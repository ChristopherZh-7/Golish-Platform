//! Deterministic whole-batch materializer for the Plan B investigation read model.
//!
//! Canonical transactions append immutable typed source snapshots and advance
//! only the source head. The projector consumes batches strictly by source
//! sequence and publishes entity/change rows plus the projection head in one
//! transaction.

mod campaigns;
mod comparison;
mod hypotheses;
mod legacy;
mod projector;
mod summary;
mod timeline;
mod types;
mod version;
mod worker;

pub use campaigns::{
    get_investigation_campaign, get_investigation_campaign_for_stage_run,
    list_investigation_campaigns, list_investigation_campaigns_for_stage_run,
};
pub use comparison::{
    compare_and_record_v1, CompareAndRecordV1Input, InvestigationComparisonSampleV1,
};
pub use hypotheses::{
    get_investigation_hypothesis, get_investigation_hypothesis_for_stage_run,
    list_investigation_hypotheses, list_investigation_hypotheses_for_stage_run,
    PLAN_B_CAPABILITY_STATE_NOT_AVAILABLE,
};
pub use legacy::{load_attempt_history, LegacyAttemptHistoryV1};
pub use projector::{
    capture_projection_head, claim_next_projection_batch, project_next_projection_batch,
    project_projection_batch, read_projection_at_head,
};
pub use summary::{read_investigation_summary, read_investigation_summary_for_stage_run};
pub use timeline::{
    read_investigation_timeline, read_investigation_timeline_for_stage_run,
    InvestigationTimelinePage, InvestigationTimelineQuery,
};
pub use types::{
    CapturedProjectionHead, InvestigationActorTopologyNode, InvestigationCampaignDetail,
    InvestigationCampaignFilters, InvestigationCampaignListItem, InvestigationCampaignListPage,
    InvestigationCampaignListQuery, InvestigationCampaignSortKey, InvestigationCoverageDenominator,
    InvestigationGenerationSummary, InvestigationHypothesisDetail, InvestigationHypothesisFilters,
    InvestigationHypothesisListItem, InvestigationHypothesisListPage,
    InvestigationHypothesisListQuery, InvestigationHypothesisSortKey,
    InvestigationLegacyProjection, InvestigationOpenObligationSummary,
    InvestigationOperationReadAuthority, InvestigationPageValidation,
    InvestigationPageValidationInput, InvestigationProjectionChange, InvestigationProjectionError,
    InvestigationProjectionResult, InvestigationReadAuthority, InvestigationSourceCensusMember,
    InvestigationStageRunReadAuthority, InvestigationStageRunSelector, InvestigationSummary,
    InvestigationTemporalReadAuthority, InvestigationWaveSummary, MaterializedProjectionEntity,
    ProjectionBatchClaim, ProjectionBatchEnqueueReceipt, ProjectionBatchReceipt,
    ProjectionProjectOutcome, ProjectionReadPage, ProjectionStaleReason,
    INVESTIGATION_PROJECTION_PAYLOAD_INVALID, INVESTIGATION_PROJECTION_STALE,
};
pub use version::{
    begin_read_snapshot, LegacyField, OperationReadAuthority, ProjectionAuthorityTimeV1,
    ProjectionHead, ProjectionItem, ProjectionPage, ProjectionTemporalStatusV1,
};
pub use worker::InvestigationProjectionWorker;

pub use crate::repo::hypothesis_legacy_projection::{
    read_legacy_attempt_projection, read_legacy_candidate_projection,
    AppendProjectionSourceBatchRow as ProjectionOutboxBatchInput,
    LegacyCompatibilityProjectionVersion, LegacyCompatibilityRead,
    LegacyCompatibilityReadDisposition, ProjectionOutboxSourceRow as ProjectionOutboxMemberInput,
    ProjectionSourceStorageV1,
};

use chrono::{DateTime, Utc};
use golish_pentest_domain::tool_truth::ToolTruthContract;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

const EMPTY_AUTHORITY_VALID_UNTIL: &str = "9999-12-31 23:59:59+00";
const INVESTIGATION_AUTHORITY_CORRUPT: &str = "INVESTIGATION_AUTHORITY_CORRUPT";
pub const INVESTIGATION_PROJECTION_NOTIFY_CHANNEL: &str = "golish_investigation_projection";

async fn ensure_registry_authority_exact_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    as_of_change_seq: i64,
) -> InvestigationProjectionResult<()> {
    let exact: bool = sqlx::query_scalar(
        r#"WITH generation_dependencies AS MATERIALIZED (
               SELECT entity.projection_body #>>
                          '{record,canonicalRedactedBody,generation_id}' AS generation_id,
                      (entity.projection_body #>>
                          '{record,canonicalRedactedBody,candidate_snapshot_id}')::UUID
                          AS snapshot_id
                 FROM investigation_projection_entity_versions entity
                WHERE entity.operation_id=$1 AND entity.entity_kind='generation'
                  AND entity.change_seq<=$2 AND entity.invalidation_reason IS NULL
           ), hypothesis_dependencies AS MATERIALIZED (
               SELECT entity.projection_body #>>
                          '{record,canonicalRedactedBody,source_generation_id}'
                          AS generation_id
                 FROM investigation_projection_entity_versions entity
                WHERE entity.operation_id=$1 AND entity.entity_kind='hypothesis'
                  AND entity.change_seq<=$2 AND entity.invalidation_reason IS NULL
           )
           SELECT
             NOT EXISTS(
                 SELECT 1 FROM hypothesis_dependencies hypothesis
                  WHERE hypothesis.generation_id IS NULL OR NOT EXISTS(
                      SELECT 1 FROM generation_dependencies generation
                       WHERE generation.generation_id=hypothesis.generation_id))
             AND NOT EXISTS(
                 SELECT 1 FROM generation_dependencies generation
                  LEFT JOIN candidate_analysis_snapshots snapshot
                    ON snapshot.snapshot_id=generation.snapshot_id
                   AND snapshot.operation_id=$1
                 WHERE snapshot.snapshot_id IS NULL OR snapshot.snapshot_status NOT IN (
                         'sealed_ready','sealed_analysis_ready_with_residuals'
                       )
                    OR snapshot.relevant_root_count<>3 OR snapshot.bundle_member_count<>3
                    OR (SELECT COUNT(*)
                          FROM candidate_analysis_snapshot_authority_bundle_members member
                         WHERE member.snapshot_id=snapshot.snapshot_id)<>3
                    OR (SELECT COUNT(DISTINCT member.root_family)
                          FROM candidate_analysis_snapshot_authority_bundle_members member
                         WHERE member.snapshot_id=snapshot.snapshot_id)<>3
                    OR NOT EXISTS(
                         SELECT 1 FROM candidate_analysis_temporal_validity_censuses census
                          WHERE census.snapshot_id=snapshot.snapshot_id
                            AND census.decision_count=(
                                SELECT COUNT(*)
                                  FROM candidate_analysis_temporal_validity_census_members decision
                                 WHERE decision.census_id=census.census_id))
                    OR EXISTS(
                         SELECT 1
                           FROM candidate_analysis_snapshot_authority_bundle_members bundle_member
                           JOIN tool_truth_authority_set_members authority_member
                             ON authority_member.authority_set_id=bundle_member.authority_set_seal_id
                           JOIN capability_execution_temporal_census_members temporal_member
                             ON temporal_member.receipt_id=authority_member.receipt_id
                           LEFT JOIN tool_truth_target_state_epoch_heads current_head
                             ON current_head.operation_id=temporal_member.target_state_operation_id
                            AND current_head.organization_id=temporal_member.target_state_organization_id
                            AND current_head.target_scope_identity_hash=
                                temporal_member.target_scope_identity_hash
                          WHERE bundle_member.snapshot_id=snapshot.snapshot_id
                            AND current_head.operation_id IS NULL)
                    OR NOT EXISTS(
                         SELECT 1
                           FROM candidate_analysis_knowledge_feed_denominators denominator
                           JOIN candidate_analysis_knowledge_feed_snapshots feed_snapshot
                             ON feed_snapshot.denominator_id=denominator.denominator_id
                            AND feed_snapshot.snapshot_id=denominator.snapshot_id
                          WHERE denominator.snapshot_id=snapshot.snapshot_id
                            AND denominator.required_member_count=feed_snapshot.member_count
                            AND feed_snapshot.member_count=(
                                SELECT COUNT(*)
                                  FROM candidate_analysis_knowledge_feed_snapshot_members member
                                 WHERE member.feed_snapshot_id=feed_snapshot.feed_snapshot_id))
                    OR NOT EXISTS(
                         SELECT 1
                           FROM candidate_operation_managed_feed_contracts contract
                           JOIN candidate_managed_feed_catalog_head catalog_head
                             ON catalog_head.singleton
                            AND catalog_head.catalog_id=contract.catalog_id
                           JOIN candidate_managed_feed_trust_store_head trust_head
                             ON trust_head.singleton
                           JOIN candidate_analysis_knowledge_feed_denominators denominator
                             ON denominator.snapshot_id=snapshot.snapshot_id
                            AND denominator.catalog_id=contract.catalog_id
                            AND denominator.catalog_hash=contract.catalog_hash
                            AND denominator.trust_store_hash=trust_head.trust_store_hash
                            AND denominator.key_revocation_epoch=trust_head.key_revocation_epoch
                          WHERE contract.operation_id=$1
                            AND contract.required_member_count=(
                                SELECT COUNT(*)
                                  FROM candidate_managed_feed_store_member_heads member_head
                                 WHERE member_head.catalog_id=contract.catalog_id))
             )"#,
    )
    .bind(operation_id)
    .bind(as_of_change_seq)
    .fetch_one(&mut **tx)
    .await?;
    if exact {
        Ok(())
    } else {
        Err(InvestigationProjectionError::Contract(
            INVESTIGATION_AUTHORITY_CORRUPT,
        ))
    }
}

pub(super) struct InvestigationProjectionReadSnapshot<'a> {
    pub(super) tx: Transaction<'a, Postgres>,
    pub(super) authority: InvestigationReadAuthority,
}

impl<'a> InvestigationProjectionReadSnapshot<'a> {
    pub(super) async fn begin(
        pool: &'a PgPool,
        operation_id: Uuid,
    ) -> InvestigationProjectionResult<Self> {
        let mut tx = pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;

        #[derive(sqlx::FromRow)]
        struct Header {
            projection_schema_version: i32,
            change_seq: i64,
            cursor_salt: Vec<u8>,
            tool_truth_contract: String,
            investigation_contract_version: String,
            investigation_rollout_mode: String,
            read_at: DateTime<Utc>,
        }

        let header = sqlx::query_as::<_, Header>(
            r#"SELECT head.projection_schema_version,head.change_seq,head.cursor_salt,
                      operation.tool_truth_contract,operation.investigation_contract_version,
                      operation.investigation_rollout_mode,
                      transaction_timestamp() AS read_at
                 FROM investigation_projection_heads head
                 JOIN operation_state operation USING(operation_id)
                WHERE head.operation_id=$1"#,
        )
        .bind(operation_id)
        .fetch_one(&mut *tx)
        .await?;
        let tool_truth_contract = ToolTruthContract::try_from(header.tool_truth_contract.as_str())
            .map_err(|_| types::invalid_payload("unknown operation Tool Truth contract"))?;
        let (investigation_contract_version, investigation_rollout_mode) =
            crate::repo::investigation_rollout::parse_frozen_pair(
                &header.investigation_contract_version,
                &header.investigation_rollout_mode,
            )
            .map_err(|_| types::invalid_payload("unknown operation Investigation contract"))?;
        crate::repo::operation_rollout::validate_joint_pair(
            tool_truth_contract,
            investigation_contract_version,
            investigation_rollout_mode,
        )
        .map_err(|_| types::invalid_payload("operation joint contract pair is invalid"))?;
        if header.projection_schema_version != 1 {
            return Err(types::invalid_payload(
                "unsupported investigation projection schema version",
            ));
        }
        if investigation_rollout_mode.policy().canonical_writer
            == golish_core::InvestigationAuthority::Registry
        {
            ensure_registry_authority_exact_on(&mut tx, operation_id, header.change_seq).await?;
        }
        let cursor_salt: [u8; 32] = header
            .cursor_salt
            .try_into()
            .map_err(|_| types::invalid_payload("projection cursor salt is not 32 bytes"))?;

        let (authority_epoch_set_hash, earliest_effective_valid_until): (String, DateTime<Utc>) =
            sqlx::query_as(
                r#"WITH generation_dependencies AS MATERIALIZED (
                       SELECT DISTINCT
                              (entity.projection_body #>>
                                  '{record,canonicalRedactedBody,candidate_snapshot_id}')::UUID
                                  AS snapshot_id
                         FROM investigation_projection_entity_versions entity
                        WHERE entity.operation_id=$1
                          AND entity.entity_kind='generation'
                          AND entity.change_seq<=$2
                   ), target_epoch_rows AS MATERIALIZED (
                       SELECT DISTINCT dependency.snapshot_id,snapshot_member.ordinal AS root_ordinal,
                              authority_member.ordinal AS authority_ordinal,
                              temporal_member.ordinal AS temporal_ordinal,
                              current_head.organization_id,
                              current_head.target_scope_identity_hash,
                              current_head.current_event_id,current_head.current_epoch,
                              current_head.row_version
                         FROM generation_dependencies dependency
                         JOIN candidate_analysis_snapshot_authority_bundle_members snapshot_member
                           ON snapshot_member.snapshot_id=dependency.snapshot_id
                         JOIN tool_truth_authority_set_members authority_member
                           ON authority_member.authority_set_id=snapshot_member.authority_set_seal_id
                         JOIN capability_execution_temporal_census_members temporal_member
                           ON temporal_member.receipt_id=authority_member.receipt_id
                         JOIN tool_truth_target_state_epoch_heads current_head
                           ON current_head.operation_id=temporal_member.target_state_operation_id
                          AND current_head.organization_id=temporal_member.target_state_organization_id
                          AND current_head.target_scope_identity_hash=
                              temporal_member.target_scope_identity_hash
                   ), authority_manifest AS (
                       SELECT jsonb_build_object(
                           'target_epochs',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                               'snapshot_id',snapshot_id,'root_ordinal',root_ordinal,
                               'authority_ordinal',authority_ordinal,
                               'temporal_ordinal',temporal_ordinal,
                               'organization_id',organization_id,
                               'target_scope_identity_hash',target_scope_identity_hash,
                               'current_event_id',current_event_id,
                               'current_epoch',current_epoch,'row_version',row_version
                           ) ORDER BY snapshot_id,root_ordinal,authority_ordinal,temporal_ordinal,
                                      target_scope_identity_hash) FROM target_epoch_rows),'[]'::JSONB),
                           'feed_trust_head',COALESCE((SELECT jsonb_build_object(
                               'trust_store_version',trust_store_version,
                               'trust_store_hash',trust_store_hash,
                               'key_revocation_epoch',key_revocation_epoch,
                               'key_revocation_epoch_hash',key_revocation_epoch_hash,
                               'head_version',head_version)
                               FROM candidate_managed_feed_trust_store_head
                              WHERE singleton AND EXISTS(SELECT 1 FROM generation_dependencies)),
                              '{}'::JSONB),
                           'feed_catalog_head',COALESCE((SELECT jsonb_build_object(
                               'catalog_id',catalog_id,'catalog_version',catalog_version,
                               'catalog_hash',catalog_hash,'head_version',head_version)
                               FROM candidate_managed_feed_catalog_head
                              WHERE singleton AND EXISTS(SELECT 1 FROM generation_dependencies)),
                              '{}'::JSONB),
                           'feed_member_heads',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                               'catalog_member_id',member_head.catalog_member_id,
                               'store_member_id',member_head.store_member_id,
                               'head_version',member_head.head_version)
                               ORDER BY member_head.catalog_member_id)
                               FROM candidate_managed_feed_store_member_heads member_head
                               JOIN candidate_operation_managed_feed_contracts contract
                                 ON contract.catalog_id=member_head.catalog_id
                              WHERE contract.operation_id=$1
                                AND EXISTS(SELECT 1 FROM generation_dependencies)),
                              '[]'::JSONB)
                       ) AS body
                   )
                   SELECT tool_truth_sha256((SELECT body::TEXT FROM authority_manifest)),
                          LEAST(
                              COALESCE((SELECT MIN(bundle.effective_valid_until)
                                  FROM generation_dependencies dependency
                                  JOIN candidate_analysis_snapshots snapshot
                                    ON snapshot.snapshot_id=dependency.snapshot_id
                                  JOIN tool_truth_authority_bundle_seals bundle
                                    ON bundle.id=snapshot.tool_truth_authority_bundle_seal_id),
                                  $3::TIMESTAMPTZ),
                              COALESCE((SELECT MIN(feed.effective_valid_until)
                                  FROM generation_dependencies dependency
                                  JOIN candidate_analysis_knowledge_feed_snapshot_members feed
                                    ON feed.snapshot_id=dependency.snapshot_id
                                 WHERE feed.effective_valid_until IS NOT NULL),
                                  $3::TIMESTAMPTZ)
                          )"#,
            )
            .bind(operation_id)
            .bind(header.change_seq)
            .bind(EMPTY_AUTHORITY_VALID_UNTIL)
            .fetch_one(&mut *tx)
            .await?;
        if authority_epoch_set_hash.len() != 71
            || !authority_epoch_set_hash.starts_with("sha256:")
            || !authority_epoch_set_hash[7..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(types::invalid_payload(
                "authority epoch exact-set hash is malformed",
            ));
        }
        if header.read_at > earliest_effective_valid_until {
            return Err(InvestigationProjectionError::Stale {
                code: INVESTIGATION_PROJECTION_STALE,
                current_change_seq: header.change_seq,
                reason: ProjectionStaleReason::TemporalCutoffExpired,
            });
        }

        Ok(Self {
            tx,
            authority: InvestigationReadAuthority {
                operation: InvestigationOperationReadAuthority {
                    operation_id,
                    tool_truth_contract: tool_truth_contract.as_str().to_owned(),
                    investigation_contract_version: investigation_contract_version
                        .as_str()
                        .to_owned(),
                    investigation_rollout_mode: investigation_rollout_mode.as_str().to_owned(),
                    cursor_salt,
                },
                temporal: InvestigationTemporalReadAuthority {
                    projection_schema_version: header.projection_schema_version,
                    as_of_change_seq: header.change_seq,
                    as_of_temporal_cutoff: header.read_at,
                    authority_epoch_set_hash,
                    earliest_effective_valid_until,
                },
            },
        })
    }

    pub(super) async fn finish(self) -> InvestigationProjectionResult<()> {
        self.tx.commit().await?;
        Ok(())
    }

    pub(super) async fn begin_for_stage_run(
        pool: &'a PgPool,
        operation_id: Uuid,
        selector: &InvestigationStageRunSelector,
    ) -> InvestigationProjectionResult<(Self, InvestigationStageRunReadAuthority)> {
        let mut snapshot = Self::begin(pool, operation_id).await?;
        let stage_run =
            validate_exact_stage_run_on(&mut snapshot.tx, operation_id, selector).await?;
        Ok((snapshot, stage_run))
    }
}

async fn validate_exact_stage_run_on(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    selector: &InvestigationStageRunSelector,
) -> InvestigationProjectionResult<InvestigationStageRunReadAuthority> {
    if operation_id.is_nil()
        || selector.stage_execution_id.is_nil()
        || selector.scope_snapshot_id.is_nil()
        || selector.stage_run_request_id.trim().is_empty()
        || selector.stage_run_request_id.len() > 512
    {
        return Err(types::invalid_payload(
            "exact Investigation stage selector is malformed",
        ));
    }
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            Uuid,
            String,
            Uuid,
            String,
            bool,
            i64,
            i64,
            i64,
            String,
        ),
    >(
        r#"SELECT authority_id,operation_id,stage_execution_id,
                  owning_stage_run_request_id,scope_snapshot_id,run_state,
                  admission_open,stop_epoch,change_seq,head_version,head_sha256
             FROM investigation_run_heads
            WHERE operation_id=$1 AND stage_execution_id=$2
              AND owning_stage_run_request_id=$3 AND scope_snapshot_id=$4
            ORDER BY authority_id
            LIMIT 2"#,
    )
    .bind(operation_id)
    .bind(selector.stage_execution_id)
    .bind(selector.stage_run_request_id.trim())
    .bind(selector.scope_snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    if rows.len() != 1 {
        return Err(types::invalid_payload(
            "exact Investigation stage selector is unavailable or ambiguous",
        ));
    }
    let (
        authority_id,
        operation_id,
        stage_execution_id,
        stage_run_request_id,
        scope_snapshot_id,
        run_state,
        admission_open,
        stop_epoch,
        change_seq,
        head_version,
        head_sha256,
    ) = rows.into_iter().next().expect("one exact stage-run row");
    Ok(InvestigationStageRunReadAuthority {
        authority_id,
        operation_id,
        stage_execution_id,
        stage_run_request_id,
        scope_snapshot_id,
        run_state,
        admission_open,
        stop_epoch,
        change_seq,
        head_version,
        head_sha256,
    })
}

fn apply_expected_page_authority(
    snapshot: &mut InvestigationProjectionReadSnapshot<'_>,
    expected: &InvestigationPageValidationInput,
) -> InvestigationProjectionResult<()> {
    if expected.as_of_temporal_cutoff > expected.earliest_effective_valid_until {
        return Err(types::invalid_payload(
            "temporal cutoff is after earliest effective valid-until",
        ));
    }
    let current = &snapshot.authority.temporal;
    if current.as_of_temporal_cutoff < expected.as_of_temporal_cutoff {
        return Err(types::invalid_payload(
            "cursor temporal cutoff is in the database future",
        ));
    }
    let stale_reason = if current.as_of_change_seq != expected.as_of_change_seq {
        Some(ProjectionStaleReason::ChangeSeqAdvanced)
    } else if current.authority_epoch_set_hash != expected.authority_epoch_set_hash {
        Some(ProjectionStaleReason::AuthorityEpochChanged)
    } else if current.as_of_temporal_cutoff > expected.earliest_effective_valid_until {
        Some(ProjectionStaleReason::TemporalCutoffExpired)
    } else {
        None
    };
    if let Some(reason) = stale_reason {
        return Err(InvestigationProjectionError::Stale {
            code: INVESTIGATION_PROJECTION_STALE,
            current_change_seq: current.as_of_change_seq,
            reason,
        });
    }
    snapshot.authority.temporal.as_of_change_seq = expected.as_of_change_seq;
    snapshot.authority.temporal.as_of_temporal_cutoff = expected.as_of_temporal_cutoff;
    snapshot.authority.temporal.authority_epoch_set_hash =
        expected.authority_epoch_set_hash.clone();
    snapshot.authority.temporal.earliest_effective_valid_until =
        expected.earliest_effective_valid_until;
    Ok(())
}

/// Revalidate a signed page snapshot before any continuation query.  A valid
/// page keeps its original temporal fields; only the current DB clock and
/// current mutable authority heads are used for the drift decision.
pub async fn validate_investigation_page(
    pool: &PgPool,
    operation_id: Uuid,
    expected: &InvestigationPageValidationInput,
) -> InvestigationProjectionResult<InvestigationPageValidation> {
    let mut snapshot = InvestigationProjectionReadSnapshot::begin(pool, operation_id).await?;
    match apply_expected_page_authority(&mut snapshot, expected) {
        Ok(()) => {
            let authority = snapshot.authority.clone();
            snapshot.finish().await?;
            Ok(InvestigationPageValidation::Current(authority))
        }
        Err(InvestigationProjectionError::Stale {
            current_change_seq, ..
        }) => {
            snapshot.finish().await?;
            Ok(InvestigationPageValidation::Stale {
                current_change_seq,
                restart_required: true,
            })
        }
        Err(error) => Err(error),
    }
}

/// Capture the operation/head/temporal authority needed to verify an opaque
/// cursor before a selector-bearing materialized query is attempted.
pub async fn capture_investigation_read_authority(
    pool: &PgPool,
    operation_id: Uuid,
) -> InvestigationProjectionResult<InvestigationReadAuthority> {
    let snapshot = InvestigationProjectionReadSnapshot::begin(pool, operation_id).await?;
    let authority = snapshot.authority.clone();
    snapshot.finish().await?;
    Ok(authority)
}

/// Capture projection and exact unified-stage authority from one RR/RO
/// snapshot. This is the only cursor bootstrap used by the six current read
/// commands; it never resolves a latest stage execution.
pub async fn capture_investigation_read_authority_for_stage_run(
    pool: &PgPool,
    operation_id: Uuid,
    selector: &InvestigationStageRunSelector,
) -> InvestigationProjectionResult<(
    InvestigationReadAuthority,
    InvestigationStageRunReadAuthority,
)> {
    let (snapshot, stage_run) =
        InvestigationProjectionReadSnapshot::begin_for_stage_run(pool, operation_id, selector)
            .await?;
    let authority = snapshot.authority.clone();
    snapshot.finish().await?;
    Ok((authority, stage_run))
}

/// Public typed enqueue seam for canonical sources outside the Hypothesis
/// finalizer. It delegates to the single source-head writer; no materialized
/// entity, legacy compatibility row, or projection head is touched here.
pub async fn enqueue_projection_batch_on(
    tx: &mut Transaction<'_, Postgres>,
    input: ProjectionOutboxBatchInput,
) -> crate::Result<ProjectionBatchEnqueueReceipt> {
    let view =
        crate::repo::hypothesis_legacy_projection::append_projection_source_batch_on(tx, input)
            .await?;
    Ok(ProjectionBatchEnqueueReceipt {
        batch_id: view.batch_id,
        operation_id: view.operation_id,
        source_batch_seq: view.source_batch_seq,
        predecessor_batch_id: view.predecessor_batch_id,
        member_count: view.member_count,
        member_set_hash: view.member_set_hash,
        replayed: view.replayed,
    })
}
