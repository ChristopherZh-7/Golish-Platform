//! CAS-safe Target Intel review cursor and verdict writes.

use std::collections::BTreeSet;

use anyhow::{bail, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

pub(super) const REVIEWER_ROLE: &str = "intel_goal_reviewer";
pub(super) const REVIEWER_KIND: &str = "target_intel_read_only_review";
pub(super) const REVIEWER_SCHEMA: &str = "intel_review.v1";
pub(super) const REVIEWER_HOST_PROMPT_VERSION: &str = "target_intel_reviewer.v1";
pub(super) const REVIEWER_HOST_PROMPT: &str = "You are a read-only Target Intel Goal reviewer. Read durable_state, observable_actions, frozen_contract, and completion_claim exactly once in host order. The durable state includes controller_work_memory: the exact same-chain plan, tool history, and checkpoint. Compare the Main AI plan and claim against Tool Truth, worker outputs, frontier dispositions, observations, attribution/reachability records, and formal Targets actually landed. observable_actions.query_receipts are the semantic query receipts; a receipt whose outcome is checked_empty terminally closes its shown pivot and intent even when result_status is Partial, because provider_status is transparent capability detail rather than an implicit unfinished direction. observable_actions.semantic_receipts are candidate-bearing observation receipts and may correctly be empty on checked-empty closure. A finished recon_search_intel call without a query_receipt may be a rejected or unauthorized pivot attempt; use controller_work_memory and the work journal to distinguish it, and do not demand evidence for a rejected attempt. completion_claim.target_count is the whole authoritative target snapshot and may include trusted pre-stage Scoping intake; it is not the Target Intel promotion count. Promotion requirements apply only to durable_state.formal_assets, and zero formal assets is valid when no owned freshly reachable candidate was discovered. PASS only when every material discovery direction is terminal and every actually promoted Target is owned plus freshly reachable. Return actionable REWORK findings with grounded evidence_refs, action_kind, and close_condition when work or landing is missing. Use NEEDS_HUMAN for an unresolved capability or scope decision only when the frozen contract or a material finding makes it necessary. evidence_refs and inherited resolution_refs must cite only frozen-bundle evidence ledger ids formatted audit:<id>; section hashes are context identities, not evidence. Every finding must use the exact closed tool shape: finding_id UUID, fingerprint, materiality, subject_refs, reason, evidence_refs, and nullable action_kind, capability_ref, close_condition. Set fingerprint to sha256: followed by 64 lowercase zeroes; the trusted host replaces it with the canonical semantic finding fingerprint and the DB independently verifies it. After reading completion_claim, decide immediately: do not recount the bundle or write a prose review. Return only intel_review.v1 through target_intel_record_review_verdict. Do not search, fetch, spawn, mutate state, reopen a controller, create a hold, or mint a pass token.";

pub(super) fn reviewer_host_prompt_sha256() -> String {
    sha256_prefixed(REVIEWER_HOST_PROMPT.as_bytes())
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TargetIntelReviewFreezeSnapshot {
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub team_plan_id: Uuid,
    pub controller_work_item_id: Uuid,
    pub controller_worker_run_id: Uuid,
    pub controller_message_chain_id: Option<Uuid>,
    pub goal_epoch_id: Uuid,
    pub goal_epoch: i64,
    pub goal_epoch_row_version: i64,
    pub plan_row_version: i64,
    pub dispatch_epoch: i64,
    pub request_epoch_preclosed: bool,
    pub controller_final_submitter_prebound: bool,
    pub runtime_mode: String,
    pub reviewer_retry_fuel: i32,
    pub operation_contract_sha256: String,
    pub state_revision: i64,
    pub action_revision: i64,
    pub evidence_high_water: i64,
    pub tool_high_water: i64,
    pub review_generation: i64,
    pub round: i32,
    pub durable_state: Value,
    pub observable_actions: Value,
    pub frozen_contract: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertFrozenTargetIntelReview {
    pub review_id: Uuid,
    pub snapshot: TargetIntelReviewFreezeSnapshot,
    pub expected_plan_row_version: i64,
    pub completion_claim: Value,
    pub material_revision_vector: Value,
    pub material_state_sha256: String,
    pub material_actions_sha256: String,
    pub durable_state_sha256: String,
    pub observable_actions_sha256: String,
    pub frozen_contract_sha256: String,
    pub completion_claim_sha256: String,
    pub bundle_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertedFrozenTargetIntelReview {
    pub reviewer_work_item_id: Option<Uuid>,
    pub replayed: bool,
}

/// Locate only the latest exact freeze request whose material vector is still
/// current. This is the stable response-loss key for a DTO that deliberately
/// carries no caller-authored request id.
pub async fn find_exact_freeze_replay(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    team_plan_id: Uuid,
    goal_epoch: i64,
    controller_work_item_id: Uuid,
    controller_worker_run_id: Uuid,
    completion_claim: &str,
) -> Result<Option<(i64, i32)>> {
    let row = sqlx::query_as::<_, (i64, i32)>(
        r#"SELECT review.review_generation, review.round
             FROM target_intel_goal_reviews review
             JOIN target_intel_goal_material_revisions material
               ON material.operation_id=review.operation_id
              AND material.organization_id=review.organization_id
            WHERE review.operation_id=$1
              AND review.organization_id=$2
              AND review.team_plan_id=$3
              AND review.goal_epoch=$4
              AND review.controller_work_item_id=$5
              AND review.controller_worker_run_id=$6
              AND review.completion_claim=jsonb_build_object(
                  'completion_claim', btrim($7::text)
              )
              AND review.status NOT IN ('stale','superseded')
              AND COALESCE(
                  (review.material_revision_vector ->> 'state_revision')::bigint, -1
              )=material.state_revision
              AND COALESCE(
                  (review.material_revision_vector ->> 'action_revision')::bigint, -1
              )=material.action_revision
              AND COALESCE(
                  (review.material_revision_vector ->> 'evidence_high_water')::bigint, -1
              )=GREATEST(material.evidence_high_water, COALESCE((
                  SELECT MAX(a.id) FROM audit_log a
                   WHERE a.audit_role='evidence'
                     AND a.run_id=review.operation_id
                     AND a.detail ->> 'organization_id'=review.organization_id::text
                     AND NOT (
                         a.action='stage_final_seal_attested'
                         AND a.source='runtime_memory_final_seal'
                         AND a.tool_name='runtime_memory_final_seal_attestation'
                         AND a.detail ->> 'kind'='stage_final_seal_attestation'
                     )
              ), 0))
              AND COALESCE(
                  (review.material_revision_vector ->> 'tool_high_water')::bigint, -1
              )=GREATEST(material.tool_high_water, (
                  SELECT COUNT(*) FROM tool_calls tool
                   WHERE tool.operation_id=review.operation_id
                     AND tool.organization_id=review.organization_id
                     AND tool.stage_run_unit_id=review.stage_run_unit_id
                     AND tool.name NOT IN (
                         'stage_team_request_intel_review',
                         'target_intel_read_review_section',
                         'target_intel_record_review_verdict',
                         'target_intel_finalize_goal_pass'
                     )
                     AND NOT EXISTS (
                         SELECT 1
                           FROM stage_worker_runs review_worker
                           JOIN stage_work_items review_item
                             ON review_item.id=review_worker.work_item_id
                          WHERE review_worker.id=tool.worker_run_id
                            AND review_worker.stage_run_unit_id=tool.stage_run_unit_id
                            AND review_item.stage_run_unit_id=tool.stage_run_unit_id
                            AND review_item.execution_profile='read_only_reviewer'
                     )
              ))
              AND NOT EXISTS (
                  SELECT 1 FROM target_intel_goal_reviews newer
                   WHERE newer.operation_id=review.operation_id
                     AND newer.organization_id=review.organization_id
                     AND newer.review_generation > review.review_generation
              )
            ORDER BY review.review_generation DESC
            LIMIT 1"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(team_plan_id)
    .bind(goal_epoch)
    .bind(controller_work_item_id)
    .bind(controller_worker_run_id)
    .bind(completion_claim)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
struct FrozenTargetIntelReviewReplayRow {
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    team_plan_id: Uuid,
    goal_epoch_id: Uuid,
    goal_epoch: i64,
    review_generation: i64,
    round: i32,
    controller_work_item_id: Uuid,
    controller_worker_run_id: Option<Uuid>,
    controller_message_chain_id: Option<Uuid>,
    reviewer_work_item_id: Option<Uuid>,
    operation_contract_sha256: String,
    material_revision_vector: Value,
    material_state_sha256: String,
    material_actions_sha256: String,
    durable_state: Value,
    durable_state_sha256: String,
    observable_actions: Value,
    observable_actions_sha256: String,
    frozen_contract: Value,
    frozen_contract_sha256: String,
    completion_claim: Value,
    completion_claim_sha256: String,
    bundle_sha256: String,
}

#[derive(Debug, sqlx::FromRow)]
struct TargetIntelReviewFreezeCasRow {
    plan_row_version: i64,
    epoch_row_version: i64,
    epoch_status: String,
    dispatch_epoch: i64,
    requests_are_open: bool,
    final_submitter_is_unbound: bool,
    controller_is_final_submitter: bool,
    state_revision: i64,
    action_revision: i64,
    evidence_high_water: i64,
    tool_high_water: i64,
    active_ordinary_work_items: i64,
    active_ordinary_workers: i64,
    active_ordinary_tools: i64,
    controller_tool_is_idle: bool,
}

pub async fn load_freeze_snapshot(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    team_plan_id: Uuid,
    controller_work_item_id: Uuid,
    controller_worker_run_id: Uuid,
    expected_goal_epoch: i64,
) -> Result<TargetIntelReviewFreezeSnapshot> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let identity = sqlx::query_as::<_, TargetIntelReviewFreezeSnapshot>(
        r#"SELECT p.operation_id, p.organization_id, p.stage_execution_id,
                  p.stage_run_unit_id, p.scope_snapshot_id, p.id AS team_plan_id,
                  e.controller_work_item_id, e.controller_worker_run_id,
                  e.controller_message_chain_id, e.id AS goal_epoch_id,
                  e.epoch AS goal_epoch, e.row_version AS goal_epoch_row_version,
                  p.row_version AS plan_row_version, p.dispatch_epoch,
                  p.requests_closed_at IS NOT NULL AS request_epoch_preclosed,
                  COALESCE(
                      p.final_submitter_worker_run_id=e.controller_worker_run_id,
                      FALSE
                  ) AS controller_final_submitter_prebound,
                  c.runtime_mode, c.reviewer_retry_fuel,
                  c.goal_contract_sha256 AS operation_contract_sha256,
                  m.state_revision, m.action_revision,
                  GREATEST(m.evidence_high_water, COALESCE((
                      SELECT MAX(a.id) FROM audit_log a
                       WHERE a.audit_role='evidence'
                         AND a.run_id=p.operation_id
                         AND a.detail ->> 'organization_id'=p.organization_id::text
                         AND NOT (
                             a.action='stage_final_seal_attested'
                             AND a.source='runtime_memory_final_seal'
                             AND a.tool_name='runtime_memory_final_seal_attestation'
                             AND a.detail ->> 'kind'='stage_final_seal_attestation'
                         )
                  ), 0)) AS evidence_high_water,
                  GREATEST(m.tool_high_water, (
                      SELECT COUNT(*) FROM tool_calls t
                       WHERE t.operation_id=p.operation_id
                         AND t.organization_id=p.organization_id
                         AND t.stage_run_unit_id=p.stage_run_unit_id
                         AND t.name NOT IN (
                             'stage_team_request_intel_review',
                             'target_intel_read_review_section',
                             'target_intel_record_review_verdict',
                             'target_intel_finalize_goal_pass'
                         )
                         AND NOT EXISTS (
                             SELECT 1
                               FROM stage_worker_runs review_worker
                               JOIN stage_work_items review_item
                                 ON review_item.id=review_worker.work_item_id
                              WHERE review_worker.id=t.worker_run_id
                                AND review_worker.stage_run_unit_id=t.stage_run_unit_id
                                AND review_item.stage_run_unit_id=t.stage_run_unit_id
                                AND review_item.execution_profile='read_only_reviewer'
                         )
                  )) AS tool_high_water,
                  COALESCE((SELECT MAX(r.review_generation) + 1
                              FROM target_intel_goal_reviews r
                             WHERE r.operation_id=p.operation_id
                               AND r.organization_id=p.organization_id), 1) AS review_generation,
                  COALESCE((SELECT MAX(r.round) + 1
                              FROM target_intel_goal_reviews r
                             WHERE r.team_plan_id=p.id), 1) AS round,
                  '{}'::jsonb AS durable_state,
                  '{}'::jsonb AS observable_actions,
                  jsonb_build_object(
                      'profile_id', c.profile_id,
                      'runtime_mode', c.runtime_mode,
                      'completion_authority', c.completion_authority,
                      'goal_contract_version', c.goal_contract_version,
                      'goal_contract_sha256', c.goal_contract_sha256,
                      'methodology_sha256', c.methodology_sha256,
                      'tool_manifest_sha256', c.tool_manifest_sha256,
                      'provider_capability_sha256', c.provider_capability_sha256,
                      'browser_policy', c.browser_policy,
                      'budget_policy', c.budget_policy,
                      'max_review_rounds', c.max_review_rounds,
                      'reviewer_retry_fuel', c.reviewer_retry_fuel,
                      'inherited_material_findings', COALESCE((
                          SELECT jsonb_agg(jsonb_build_object(
                              'finding_id', finding.id,
                              'fingerprint', finding.fingerprint,
                              'materiality', finding.materiality,
                              'subject_refs', finding.subject_refs,
                              'reason', finding.reason,
                              'recommended_action', finding.recommended_action,
                              'close_condition', finding.close_condition
                          ) ORDER BY prior.review_generation, finding.id)
                            FROM target_intel_goal_reviews prior
                            JOIN target_intel_goal_review_findings finding
                              ON finding.review_id=prior.id
                           WHERE prior.operation_id=p.operation_id
                             AND prior.organization_id=p.organization_id
                             AND finding.materiality IN ('critical','major')
                             AND NOT EXISTS (
                                 SELECT 1
                                   FROM target_intel_goal_review_finding_resolutions resolution
                                   JOIN target_intel_goal_reviews resolution_review
                                     ON resolution_review.id=resolution.review_id
                                  WHERE resolution.finding_id=finding.id
                                    AND resolution.disposition='resolved'
                             )
                      ), '[]'::jsonb)
                  ) AS frozen_contract
             FROM stage_team_plans p
             JOIN target_intel_goal_epochs e
               ON e.team_plan_id=p.id AND e.epoch=$8
             JOIN target_intel_goal_operation_contracts c
               ON c.operation_id=p.operation_id
             JOIN target_intel_goal_material_revisions m
               ON m.operation_id=p.operation_id AND m.organization_id=p.organization_id
            WHERE p.id=$5 AND p.operation_id=$1 AND p.organization_id=$2
              AND p.stage_execution_id=$3 AND p.stage_run_unit_id=$4
              AND e.controller_work_item_id=$6
              AND e.controller_worker_run_id=$7
              AND e.status='open' AND p.dispatch_epoch=e.epoch
              AND (
                    (p.requests_closed_at IS NULL
                     AND p.final_submitter_worker_run_id IS NULL)
                 OR (p.requests_closed_at IS NOT NULL
                     AND p.final_submitter_worker_run_id=e.controller_worker_run_id
                     AND EXISTS (
                         SELECT 1 FROM stage_deliverable_submissions submission
                          WHERE submission.operation_id=p.operation_id
                            AND submission.stage_execution_id=p.stage_execution_id
                            AND submission.stage_run_unit_id=p.stage_run_unit_id
                            AND submission.organization_id=p.organization_id
                            AND submission.worker_run_id=e.controller_worker_run_id
                            AND submission.stage_kind='target_intel'
                     ))
              )"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(team_plan_id)
    .bind(controller_work_item_id)
    .bind(controller_worker_run_id)
    .bind(expected_goal_epoch)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_FREEZE_AUTHORITY_MISSING"))?;
    if !matches!(
        identity.runtime_mode.as_str(),
        "observe_shadow" | "advisory_rework" | "intel_goal_v1"
    ) {
        bail!("TARGET_INTEL_REVIEW_RUNTIME_MODE_INVALID");
    }
    let (durable_state, observable_actions) = load_review_material(
        &mut tx,
        operation_id,
        organization_id,
        stage_run_unit_id,
        team_plan_id,
    )
    .await?;
    let frozen_contract =
        load_review_frozen_contract(&mut tx, operation_id, organization_id).await?;
    tx.commit().await?;
    Ok(TargetIntelReviewFreezeSnapshot {
        durable_state,
        observable_actions,
        frozen_contract,
        ..identity
    })
}

async fn load_review_material(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_run_unit_id: Uuid,
    team_plan_id: Uuid,
) -> Result<(Value, Value)> {
    let durable_state: Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
               'company_identity', COALESCE((
                   SELECT jsonb_build_object(
                       'receipt_id', identity.id,
                       'canonical_legal_name', identity.canonical_legal_name,
                       'aliases', identity.aliases,
                       'brands', identity.brands,
                       'registration_identifiers', identity.registration_identifiers,
                       'confirmation_method', identity.confirmation_method,
                       'identity_sha256', identity.identity_sha256,
                       'scope_policy_sha256', identity.scope_policy_sha256,
                       'evidence_refs', identity.evidence_refs,
                       'artifact_refs', identity.artifact_refs
                   )
                     FROM target_intel_goal_company_identity_bindings binding
                     JOIN scoping_company_identity_receipts identity
                       ON identity.id=binding.company_identity_receipt_id
                      AND identity.operation_id=binding.operation_id
                      AND identity.organization_id=binding.organization_id
                    WHERE binding.operation_id=$1 AND binding.organization_id=$2
               ), '{}'::jsonb),
               'controller_work_memory', COALESCE((
                   SELECT jsonb_build_object(
                       'goal_epoch', epoch.epoch,
                       'message_chain_id', epoch.controller_message_chain_id,
                       'worker_run_id', epoch.controller_worker_run_id,
                       'checkpoint_version', worker.checkpoint_version,
                       'checkpoint', worker.checkpoint,
                       'messages', chain.chain
                   )
                     FROM target_intel_goal_epochs epoch
                     JOIN stage_worker_runs worker
                       ON worker.id=epoch.controller_worker_run_id
                     JOIN message_chains chain
                       ON chain.id=epoch.controller_message_chain_id
                      AND chain.task_id=epoch.operation_id
                    WHERE epoch.operation_id=$1
                      AND epoch.organization_id=$2
                      AND epoch.team_plan_id=$3
                    ORDER BY epoch.epoch DESC
                    LIMIT 1
               ), jsonb_build_object(
                   'goal_epoch', NULL,
                   'message_chain_id', NULL,
                   'worker_run_id', NULL,
                   'checkpoint_version', NULL,
                   'checkpoint', '{}'::jsonb,
                   'messages', '[]'::jsonb
               )),
               'work_journal', COALESCE((
                   SELECT jsonb_agg(jsonb_build_object(
                       'id', journal.id, 'goal_epoch', journal.goal_epoch,
                       'ordinal', journal.ordinal, 'entry_kind', journal.entry_kind,
                       'payload', journal.payload, 'related_frontier_refs', journal.related_frontier_refs,
                       'evidence_refs', journal.evidence_refs, 'tool_call_refs', journal.tool_call_refs,
                       'observation_refs', journal.observation_refs, 'entry_sha256', journal.entry_sha256
                   ) ORDER BY journal.goal_epoch, journal.ordinal)
                     FROM target_intel_goal_work_journal_entries journal
                    WHERE journal.operation_id=$1 AND journal.organization_id=$2
                      AND journal.team_plan_id=$3
               ), '[]'::jsonb),
               'frontier', COALESCE((
                   SELECT jsonb_agg(jsonb_build_object(
                       'id', f.id, 'semantic_pivot_key', f.semantic_pivot_key,
                       'pivot_kind', f.pivot_kind, 'pivot_value_sha256', f.pivot_value_sha256,
                       'intent', f.intent, 'materiality', f.materiality, 'status', f.status,
                       'terminal_refs', f.terminal_refs, 'capability_ref', f.capability_ref,
                       'reason', f.reason, 'row_version', f.row_version,
                       'waiver', (SELECT jsonb_build_object(
                           'id', waiver.id, 'authority_kind', waiver.authority_kind,
                           'authority_ref', waiver.authority_ref,
                           'evidence_refs', waiver.evidence_refs, 'reason', waiver.reason,
                           'expected_frontier_row_version', waiver.expected_frontier_row_version
                       ) FROM target_intel_goal_frontier_waivers waiver
                          WHERE waiver.frontier_id=f.id)
                   ) ORDER BY f.semantic_pivot_key)
                     FROM target_intel_goal_frontier_v2 f
                    WHERE f.operation_id=$1 AND f.organization_id=$2
               ), '[]'::jsonb),
               'asset_observations', COALESCE((
                   SELECT jsonb_agg(jsonb_build_object(
                       'id', observation.id, 'asset_kind', observation.asset_kind,
                       'canonical_value', observation.canonical_value,
                       'canonical_identity_sha256', observation.canonical_identity_sha256,
                       'provider_id', observation.provider_id,
                       'stable_query_key', observation.stable_query_key,
                       'artifact_ref', observation.artifact_ref,
                       'artifact_sha256', observation.artifact_sha256,
                       'typed_core', observation.typed_core,
                       'provider_fields', observation.provider_fields,
                       'provider_metadata', observation.provider_metadata,
                       'attribution_disposition', observation.attribution_disposition,
                       'attribution_method', observation.attribution_method,
                       'attribution_basis', observation.attribution_basis,
                       'reachability_state', observation.reachability_state,
                       'reachability_method', observation.reachability_method,
                       'reachability_checked_at', observation.reachability_checked_at,
                       'reachability_valid_until', observation.reachability_valid_until,
                       'promotion_target_id', observation.promotion_target_id,
                       'row_version', observation.row_version
                   ) ORDER BY observation.stable_observation_key)
                     FROM target_intel_asset_observations observation
                    WHERE observation.operation_id=$1 AND observation.organization_id=$2
                      AND observation.team_plan_id=$3
               ), '[]'::jsonb),
               'formal_assets', COALESCE((
                   SELECT jsonb_agg(jsonb_build_object(
                       'observation_id', observation.id, 'target_id', target.id,
                       'target_type', target.target_type::text, 'value', target.value,
                       'scope', target.scope::text, 'source', target.source,
                       'liveness_state', target.liveness_state,
                       'liveness_checked_at', target.liveness_checked_at
                   ) ORDER BY target.id)
                     FROM target_intel_asset_observations observation
                     JOIN targets target ON target.id=observation.promotion_target_id
                    WHERE observation.operation_id=$1 AND observation.organization_id=$2
                      AND observation.team_plan_id=$3
               ), '[]'::jsonb),
               'worker_outputs', COALESCE((
                   SELECT jsonb_agg(jsonb_build_object(
                       'work_item_id', w.id, 'kind', w.kind, 'stable_key', w.stable_key,
                       'role', w.role, 'status', w.status,
                       'input_manifest_hash', w.input_manifest_hash,
                       'output_schema', w.output_schema,
                       'business_disposition', o.business_disposition,
                       'output_hash', o.output_hash, 'canonical_fact_refs', o.canonical_fact_refs,
                       'evidence_ids', to_jsonb(o.evidence_ids),
                       'checked_empty_cells', o.checked_empty_cells,
                       'blocker_codes', to_jsonb(o.blocker_codes)
                   ) ORDER BY w.stable_key)
                     FROM stage_work_items w
                     LEFT JOIN stage_worker_outputs o ON o.work_item_id=w.id
                    WHERE w.team_plan_id=$3
                      AND w.execution_profile<>'read_only_reviewer'
               ), '[]'::jsonb),
               'human_fulfillments', COALESCE((
                   SELECT jsonb_agg(jsonb_build_object(
                       'fulfillment_id', fulfillment.id, 'hold_id', hold.id,
                       'requirement_kind', hold.requirement_kind,
                       'fulfillment_kind', fulfillment.fulfillment_kind,
                       'authority_ref', fulfillment.authority_ref,
                       'material_input', fulfillment.material_input
                   ) ORDER BY fulfillment.created_at, fulfillment.id)
                     FROM target_intel_goal_hold_fulfillments fulfillment
                     JOIN target_intel_goal_holds hold ON hold.id=fulfillment.hold_id
                     JOIN target_intel_goal_reviews review ON review.id=hold.review_id
                    WHERE review.operation_id=$1 AND review.organization_id=$2
               ), '[]'::jsonb)
           )"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(team_plan_id)
    .fetch_one(&mut **tx)
    .await?;
    let observable_actions: Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
               'query_receipts', COALESCE((
                   SELECT jsonb_agg(jsonb_build_object(
                       'evidence_ref', 'audit:' || a.id::text,
                       'audit_id', a.id,
                       'outcome', a.evidence_outcome,
                       'producer_worker_run_id', a.detail ->> 'producer_worker_run_id',
                       'producer_tool_call_id', a.detail ->> 'producer_tool_call_id',
                       'provider_run_id', a.detail ->> 'provider_run_id',
                       'pivot_kind', a.detail ->> 'pivot_kind',
                       'pivot_value_sha256', a.detail ->> 'pivot_value_sha256',
                       'result_status', a.detail -> 'result_status',
                       'provider_status', a.detail -> 'provider_status',
                       'technique_status', a.detail -> 'technique_status',
                       'counts', a.detail -> 'counts'
                   ) ORDER BY a.id)
                     FROM audit_log a
                     JOIN tool_calls tool
                       ON tool.id=CASE
                           WHEN a.detail ->> 'producer_tool_call_id'
                                ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                           THEN (a.detail ->> 'producer_tool_call_id')::uuid
                           ELSE NULL
                       END
                      AND tool.operation_id=$1
                      AND tool.organization_id=$2
                      AND tool.stage_run_unit_id=$3
                      AND tool.worker_run_id::text=a.detail ->> 'producer_worker_run_id'
                      AND tool.name='recon_search_intel'
                      AND tool.status='finished'
                    WHERE a.details='intel.semantic_query_receipt.v1'
                      AND a.audit_role='evidence'
                      AND a.run_id=$1
                      AND a.detail ->> 'organization_id'=$2::text
               ), '[]'::jsonb),
               'semantic_receipts', COALESCE((
                   SELECT jsonb_agg(jsonb_build_object(
                       'audit_id', a.id, 'status', a.status,
                       'stable_query_key', a.detail ->> 'stable_query_key',
                       'artifact_ref', a.detail ->> 'artifact_ref',
                       'artifact_sha256', a.detail ->> 'artifact_sha256',
                       'evidence_ref', a.detail ->> 'evidence_ref'
                   ) ORDER BY a.id)
                     FROM audit_log a
                    WHERE a.details='intel.semantic_pivot_receipt.v1'
                      AND a.source IN ('target_intel_goal_shadow','target_intel_goal')
                      AND a.detail ->> 'operation_id'=$1::text
                      AND a.detail ->> 'organization_id'=$2::text
               ), '[]'::jsonb),
               'tool_calls', COALESCE((
                   SELECT jsonb_agg(jsonb_build_object(
                       'id', t.id, 'name', t.name, 'status', t.status,
                       'worker_run_id', t.worker_run_id,
                       'attempt_epoch', t.attempt_epoch
                   ) ORDER BY t.created_at, t.id)
                     FROM tool_calls t
                    WHERE t.operation_id=$1 AND t.organization_id=$2
                      AND t.stage_run_unit_id=$3
                      AND t.name NOT IN (
                          'stage_team_request_intel_review',
                          'target_intel_read_review_section',
                          'target_intel_record_review_verdict',
                          'target_intel_finalize_goal_pass'
                      )
                      AND NOT EXISTS (
                          SELECT 1
                            FROM stage_worker_runs review_worker
                            JOIN stage_work_items review_item
                              ON review_item.id=review_worker.work_item_id
                           WHERE review_worker.id=t.worker_run_id
                             AND review_worker.stage_run_unit_id=t.stage_run_unit_id
                             AND review_item.stage_run_unit_id=t.stage_run_unit_id
                             AND review_item.execution_profile='read_only_reviewer'
                      )
               ), '[]'::jsonb),
               'promotion_events', COALESCE((
                   SELECT jsonb_agg(jsonb_build_object(
                       'id', event.id, 'observation_id', event.observation_id,
                       'expected_row_version', event.expected_row_version,
                       'after_state', event.after_state,
                       'evidence_refs', event.evidence_refs,
                       'tool_call_refs', event.tool_call_refs
                   ) ORDER BY event.created_at, event.id)
                     FROM target_intel_asset_observation_events event
                    WHERE event.operation_id=$1 AND event.organization_id=$2
                      AND event.event_kind='promotion'
               ), '[]'::jsonb)
           )"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(stage_run_unit_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok((durable_state, observable_actions))
}

async fn load_review_frozen_contract(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    operation_id: Uuid,
    organization_id: Uuid,
) -> Result<Value> {
    sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
               'profile_id', c.profile_id,
               'runtime_mode', c.runtime_mode,
               'completion_authority', c.completion_authority,
               'goal_contract_version', c.goal_contract_version,
               'goal_contract_sha256', c.goal_contract_sha256,
               'methodology_sha256', c.methodology_sha256,
               'tool_manifest_sha256', c.tool_manifest_sha256,
               'provider_capability_sha256', c.provider_capability_sha256,
               'browser_policy', c.browser_policy,
               'budget_policy', c.budget_policy,
               'max_review_rounds', c.max_review_rounds,
               'reviewer_retry_fuel', c.reviewer_retry_fuel,
               'company_identity', COALESCE((
                   SELECT jsonb_build_object(
                       'receipt_id', binding.company_identity_receipt_id,
                       'identity_sha256', binding.company_identity_sha256,
                       'scope_policy_sha256', binding.scope_policy_sha256
                   )
                     FROM target_intel_goal_company_identity_bindings binding
                    WHERE binding.operation_id=c.operation_id
                      AND binding.organization_id=$2
               ), '{}'::jsonb),
               'inherited_material_findings', COALESCE((
                   SELECT jsonb_agg(jsonb_build_object(
                       'finding_id', finding.id,
                       'fingerprint', finding.fingerprint,
                       'materiality', finding.materiality,
                       'subject_refs', finding.subject_refs,
                       'reason', finding.reason,
                       'recommended_action', finding.recommended_action,
                       'close_condition', finding.close_condition
                   ) ORDER BY prior.review_generation, finding.id)
                     FROM target_intel_goal_reviews prior
                     JOIN target_intel_goal_review_findings finding
                       ON finding.review_id=prior.id
                    WHERE prior.operation_id=c.operation_id
                      AND prior.organization_id=$2
                      AND finding.materiality IN ('critical','major')
                      AND NOT EXISTS (
                          SELECT 1
                            FROM target_intel_goal_review_finding_resolutions resolution
                            JOIN target_intel_goal_reviews resolution_review
                              ON resolution_review.id=resolution.review_id
                           WHERE resolution.finding_id=finding.id
                             AND resolution.disposition='resolved'
                      )
               ), '[]'::jsonb)
           )
             FROM target_intel_goal_operation_contracts c
            WHERE c.operation_id=$1"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(Into::into)
}

pub async fn insert_frozen_review(
    pool: &PgPool,
    input: &InsertFrozenTargetIntelReview,
) -> Result<InsertedFrozenTargetIntelReview> {
    let snapshot = &input.snapshot;
    let expected_vector = serde_json::json!({
        "state_revision": snapshot.state_revision,
        "action_revision": snapshot.action_revision,
        "evidence_high_water": snapshot.evidence_high_water,
        "tool_high_water": snapshot.tool_high_water,
    });
    if input.review_id.is_nil()
        || input.expected_plan_row_version != snapshot.plan_row_version
        || input.material_revision_vector != expected_vector
        || canonical_sha256(&snapshot.durable_state) != input.durable_state_sha256
        || canonical_sha256(&snapshot.observable_actions) != input.observable_actions_sha256
        || canonical_sha256(&snapshot.frozen_contract) != input.frozen_contract_sha256
        || canonical_sha256(&input.completion_claim) != input.completion_claim_sha256
        || input.material_state_sha256 != input.durable_state_sha256
        || input.material_actions_sha256 != input.observable_actions_sha256
        || !is_prefixed_sha256(&input.bundle_sha256)
        || recompute_review_bundle_sha256(input)? != input.bundle_sha256
    {
        bail!("TARGET_INTEL_REVIEW_FREEZE_EXPECTATION_INVALID");
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await?;
    if let Some(persisted) = sqlx::query_as::<_, FrozenTargetIntelReviewReplayRow>(
        r#"SELECT operation_id,organization_id,stage_execution_id,
                  stage_run_unit_id,scope_snapshot_id,team_plan_id,
                  goal_epoch_id,goal_epoch,review_generation,round,
                  controller_work_item_id,controller_worker_run_id,
                  controller_message_chain_id,reviewer_work_item_id,
                  operation_contract_sha256,material_revision_vector,
                  material_state_sha256,material_actions_sha256,durable_state,
                  durable_state_sha256,observable_actions,observable_actions_sha256,
                  frozen_contract,frozen_contract_sha256,completion_claim,
                  completion_claim_sha256,bundle_sha256
             FROM target_intel_goal_reviews WHERE id=$1 FOR UPDATE"#,
    )
    .bind(input.review_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let expected = FrozenTargetIntelReviewReplayRow {
            operation_id: snapshot.operation_id,
            organization_id: snapshot.organization_id,
            stage_execution_id: snapshot.stage_execution_id,
            stage_run_unit_id: snapshot.stage_run_unit_id,
            scope_snapshot_id: snapshot.scope_snapshot_id,
            team_plan_id: snapshot.team_plan_id,
            goal_epoch_id: snapshot.goal_epoch_id,
            goal_epoch: snapshot.goal_epoch,
            review_generation: snapshot.review_generation,
            round: snapshot.round,
            controller_work_item_id: snapshot.controller_work_item_id,
            controller_worker_run_id: Some(snapshot.controller_worker_run_id),
            controller_message_chain_id: snapshot.controller_message_chain_id,
            reviewer_work_item_id: if snapshot.runtime_mode == "observe_shadow" {
                None
            } else {
                Some(deterministic_child_id(
                    input.review_id,
                    b"reviewer-work-item",
                ))
            },
            operation_contract_sha256: snapshot.operation_contract_sha256.clone(),
            material_revision_vector: input.material_revision_vector.clone(),
            material_state_sha256: input.material_state_sha256.clone(),
            material_actions_sha256: input.material_actions_sha256.clone(),
            durable_state: snapshot.durable_state.clone(),
            durable_state_sha256: input.durable_state_sha256.clone(),
            observable_actions: snapshot.observable_actions.clone(),
            observable_actions_sha256: input.observable_actions_sha256.clone(),
            frozen_contract: snapshot.frozen_contract.clone(),
            frozen_contract_sha256: input.frozen_contract_sha256.clone(),
            completion_claim: input.completion_claim.clone(),
            completion_claim_sha256: input.completion_claim_sha256.clone(),
            bundle_sha256: input.bundle_sha256.clone(),
        };
        if persisted != expected {
            bail!("TARGET_INTEL_REVIEW_FREEZE_REPLAY_MISMATCH");
        }
        tx.commit().await?;
        return Ok(InsertedFrozenTargetIntelReview {
            reviewer_work_item_id: expected.reviewer_work_item_id,
            replayed: true,
        });
    }
    let current = sqlx::query_as::<_, TargetIntelReviewFreezeCasRow>(
        r#"SELECT p.row_version AS plan_row_version,
                  e.row_version AS epoch_row_version,e.status AS epoch_status,
                  p.dispatch_epoch,p.requests_closed_at IS NULL AS requests_are_open,
                  p.final_submitter_worker_run_id IS NULL AS final_submitter_is_unbound,
                  COALESCE(
                      p.final_submitter_worker_run_id=e.controller_worker_run_id,
                      FALSE
                  ) AS controller_is_final_submitter,
                  m.state_revision, m.action_revision,
                  GREATEST(m.evidence_high_water, COALESCE((
                      SELECT MAX(a.id) FROM audit_log a
                       WHERE a.audit_role='evidence'
                         AND a.run_id=p.operation_id
                         AND a.detail ->> 'organization_id'=p.organization_id::text
                         AND NOT (
                             a.action='stage_final_seal_attested'
                             AND a.source='runtime_memory_final_seal'
                             AND a.tool_name='runtime_memory_final_seal_attestation'
                             AND a.detail ->> 'kind'='stage_final_seal_attestation'
                         )
                  ), 0)) AS evidence_high_water,
                  GREATEST(m.tool_high_water, (
                      SELECT COUNT(*) FROM tool_calls t
                       WHERE t.operation_id=p.operation_id
                         AND t.organization_id=p.organization_id
                         AND t.stage_run_unit_id=p.stage_run_unit_id
                         AND t.name NOT IN (
                             'stage_team_request_intel_review',
                             'target_intel_read_review_section',
                             'target_intel_record_review_verdict',
                             'target_intel_finalize_goal_pass'
                         )
                         AND NOT EXISTS (
                             SELECT 1
                               FROM stage_worker_runs review_worker
                               JOIN stage_work_items review_item
                                 ON review_item.id=review_worker.work_item_id
                              WHERE review_worker.id=t.worker_run_id
                                AND review_worker.stage_run_unit_id=t.stage_run_unit_id
                                AND review_item.stage_run_unit_id=t.stage_run_unit_id
                                AND review_item.execution_profile='read_only_reviewer'
                         )
                  )) AS tool_high_water,
                  (SELECT COUNT(*) FROM stage_work_items item
                    WHERE item.team_plan_id=p.id
                      AND item.id<>e.controller_work_item_id
                      AND item.execution_profile<>'read_only_reviewer'
                      AND item.status IN (
                          'queued','claimed','running','waiting_dependency',
                          'retry_pending','recovery_required'
                      )) AS active_ordinary_work_items,
                  (SELECT COUNT(*) FROM stage_worker_runs worker
                    JOIN stage_work_items item ON item.id=worker.work_item_id
                   WHERE worker.stage_run_unit_id=p.stage_run_unit_id
                     AND worker.id<>e.controller_worker_run_id
                     AND item.execution_profile<>'read_only_reviewer'
                     AND worker.status IN (
                         'queued','running','waiting_background',
                         'gate_blocked','recovery_required'
                     )) AS active_ordinary_workers,
                  (SELECT COUNT(*) FROM stage_worker_runs worker
                    JOIN stage_work_items item ON item.id=worker.work_item_id
                   WHERE worker.stage_run_unit_id=p.stage_run_unit_id
                     AND worker.id<>e.controller_worker_run_id
                     AND item.execution_profile<>'read_only_reviewer'
                     AND worker.active_tool_call_id IS NOT NULL)
                      AS active_ordinary_tools,
                  (SELECT worker.active_tool_call_id IS NULL
                     FROM stage_worker_runs worker
                    WHERE worker.id=e.controller_worker_run_id
                      AND worker.work_item_id=e.controller_work_item_id
                      AND worker.status IN (
                          'running','waiting_background','gate_blocked'
                      )) AS controller_tool_is_idle
             FROM stage_team_plans p
             JOIN target_intel_goal_epochs e
               ON e.team_plan_id=p.id AND e.id=$2
             JOIN target_intel_goal_material_revisions m
               ON m.operation_id=p.operation_id AND m.organization_id=p.organization_id
            WHERE p.id=$1 AND p.operation_id=$3 AND p.organization_id=$4
              AND p.stage_execution_id=$5 AND p.stage_run_unit_id=$6
              AND p.scope_snapshot_id=$7
              AND e.epoch=$8 AND p.dispatch_epoch=e.epoch
            FOR UPDATE OF p,e,m"#,
    )
    .bind(snapshot.team_plan_id)
    .bind(snapshot.goal_epoch_id)
    .bind(snapshot.operation_id)
    .bind(snapshot.organization_id)
    .bind(snapshot.stage_execution_id)
    .bind(snapshot.stage_run_unit_id)
    .bind(snapshot.scope_snapshot_id)
    .bind(snapshot.goal_epoch)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_FREEZE_AUTHORITY_MISSING"))?;
    let prebound_final_submission =
        snapshot.request_epoch_preclosed && snapshot.controller_final_submitter_prebound;
    if snapshot.request_epoch_preclosed != snapshot.controller_final_submitter_prebound
        || current.plan_row_version != snapshot.plan_row_version
        || current.epoch_row_version != snapshot.goal_epoch_row_version
        || current.epoch_status != "open"
        || current.dispatch_epoch != snapshot.goal_epoch
        || current.requests_are_open == snapshot.request_epoch_preclosed
        || current.final_submitter_is_unbound == snapshot.controller_final_submitter_prebound
        || (prebound_final_submission && !current.controller_is_final_submitter)
        || current.state_revision != snapshot.state_revision
        || current.action_revision != snapshot.action_revision
        || current.evidence_high_water != snapshot.evidence_high_water
        || current.tool_high_water != snapshot.tool_high_water
        || current.active_ordinary_work_items != 0
        || current.active_ordinary_workers != 0
        || current.active_ordinary_tools != 0
        || !current.controller_tool_is_idle
    {
        bail!("TARGET_INTEL_REVIEW_FREEZE_CAS_FAILED");
    }
    let (current_durable_state, current_observable_actions) = load_review_material(
        &mut tx,
        snapshot.operation_id,
        snapshot.organization_id,
        snapshot.stage_run_unit_id,
        snapshot.team_plan_id,
    )
    .await?;
    let current_frozen_contract =
        load_review_frozen_contract(&mut tx, snapshot.operation_id, snapshot.organization_id)
            .await?;
    if current_durable_state != snapshot.durable_state
        || current_observable_actions != snapshot.observable_actions
        || current_frozen_contract != snapshot.frozen_contract
    {
        bail!("TARGET_INTEL_REVIEW_FREEZE_MATERIAL_DRIFT");
    }
    let reviewer_work_item_id = if snapshot.runtime_mode == "observe_shadow" {
        None
    } else {
        let work_item_id = deterministic_child_id(input.review_id, b"reviewer-work-item");
        sqlx::query(
            r#"INSERT INTO target_intel_goal_review_freeze_authorities(
                   review_id,reviewer_work_item_id,operation_id,organization_id,
                   stage_execution_id,stage_run_unit_id,scope_snapshot_id,
                   team_plan_id,goal_epoch_id,goal_epoch,source_plan_row_version,
                   source_epoch_row_version,bundle_sha256,status
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,'building')"#,
        )
        .bind(input.review_id)
        .bind(work_item_id)
        .bind(snapshot.operation_id)
        .bind(snapshot.organization_id)
        .bind(snapshot.stage_execution_id)
        .bind(snapshot.stage_run_unit_id)
        .bind(snapshot.scope_snapshot_id)
        .bind(snapshot.team_plan_id)
        .bind(snapshot.goal_epoch_id)
        .bind(snapshot.goal_epoch)
        .bind(snapshot.plan_row_version)
        .bind(snapshot.goal_epoch_row_version)
        .bind(&input.bundle_sha256)
        .execute(&mut *tx)
        .await?;
        if !prebound_final_submission {
            let closed = sqlx::query(
                r#"UPDATE stage_team_plans
                      SET requests_closed_at=NOW(),row_version=row_version+1,updated_at=NOW()
                    WHERE id=$1 AND row_version=$2 AND dispatch_epoch=$3
                      AND requests_closed_at IS NULL
                      AND final_submitter_worker_run_id IS NULL"#,
            )
            .bind(snapshot.team_plan_id)
            .bind(snapshot.plan_row_version)
            .bind(snapshot.goal_epoch)
            .execute(&mut *tx)
            .await?;
            if closed.rows_affected() != 1 {
                bail!("TARGET_INTEL_REVIEW_FREEZE_PLAN_CAS_FAILED");
            }
        }
        let sealed = sqlx::query(
            r#"UPDATE target_intel_goal_epochs
                  SET status='sealed_for_review',sealed_at=NOW(),row_version=row_version+1
                WHERE id=$1 AND row_version=$2 AND status='open'"#,
        )
        .bind(snapshot.goal_epoch_id)
        .bind(snapshot.goal_epoch_row_version)
        .execute(&mut *tx)
        .await?;
        if sealed.rows_affected() != 1 {
            bail!("TARGET_INTEL_REVIEW_FREEZE_EPOCH_CAS_FAILED");
        }
        let prompt_sha256 = reviewer_host_prompt_sha256();
        let retry_fuel = snapshot.reviewer_retry_fuel.clamp(0, 31);
        let inserted = sqlx::query(
            r#"INSERT INTO stage_work_items (
                   id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                   scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,
                   input_manifest_hash,input_refs,required_for_barrier,conflict_key,priority,
                   status,attempt_policy,budget,output_schema,created_by,
                   execution_profile,terminal_contract,display_name,task_prompt_sha256,
                   host_prompt_version,host_prompt_sha256
               ) VALUES (
                   $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,false,NULL,-100,
                   'queued',$14,$15,$16,'target_intel_review_freeze',
                   'read_only_reviewer','intel_review_v1','Target Intel read-only reviewer',
                   $17,$18,$19
               ) ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(work_item_id)
        .bind(snapshot.team_plan_id)
        .bind(snapshot.operation_id)
        .bind(snapshot.stage_execution_id)
        .bind(snapshot.stage_run_unit_id)
        .bind(snapshot.scope_snapshot_id)
        .bind(snapshot.organization_id)
        .bind(snapshot.dispatch_epoch)
        .bind(REVIEWER_KIND)
        .bind(format!("target-intel-review:{}", input.review_id))
        .bind(REVIEWER_ROLE)
        .bind(&input.bundle_sha256)
        .bind(serde_json::json!([{
            "kind": "target_intel_review",
            "review_id": input.review_id,
            "bundle_sha256": input.bundle_sha256,
            "host_prompt": REVIEWER_HOST_PROMPT,
            "host_prompt_version": REVIEWER_HOST_PROMPT_VERSION,
        }]))
        .bind(serde_json::json!({"max_attempts": retry_fuel + 1}))
        .bind(serde_json::json!({"max_sections": 4, "max_verdicts": 1}))
        .bind(REVIEWER_SCHEMA)
        .bind(&prompt_sha256)
        .bind(REVIEWER_HOST_PROMPT_VERSION)
        .bind(&prompt_sha256)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            bail!("TARGET_INTEL_REVIEWER_WORK_ITEM_INSERT_FAILED");
        }
        Some(work_item_id)
    };
    let inserted = sqlx::query(
        r#"INSERT INTO target_intel_goal_reviews (
               id, operation_id, organization_id, stage_execution_id,
               stage_run_unit_id, scope_snapshot_id, team_plan_id,
               goal_epoch_id, goal_epoch, review_generation, round,
               controller_work_item_id, controller_worker_run_id,
               controller_message_chain_id, reviewer_work_item_id,
               operation_contract_sha256,
               material_revision_vector, material_state_sha256,
               material_actions_sha256, durable_state, durable_state_sha256,
               observable_actions, observable_actions_sha256,
               frozen_contract, frozen_contract_sha256,
               completion_claim, completion_claim_sha256, bundle_sha256,
               status, frozen_at
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
               $17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,
               'frozen',NOW()
           )"#,
    )
    .bind(input.review_id)
    .bind(snapshot.operation_id)
    .bind(snapshot.organization_id)
    .bind(snapshot.stage_execution_id)
    .bind(snapshot.stage_run_unit_id)
    .bind(snapshot.scope_snapshot_id)
    .bind(snapshot.team_plan_id)
    .bind(snapshot.goal_epoch_id)
    .bind(snapshot.goal_epoch)
    .bind(snapshot.review_generation)
    .bind(snapshot.round)
    .bind(snapshot.controller_work_item_id)
    .bind(snapshot.controller_worker_run_id)
    .bind(snapshot.controller_message_chain_id)
    .bind(reviewer_work_item_id)
    .bind(&snapshot.operation_contract_sha256)
    .bind(&input.material_revision_vector)
    .bind(&input.material_state_sha256)
    .bind(&input.material_actions_sha256)
    .bind(&snapshot.durable_state)
    .bind(&input.durable_state_sha256)
    .bind(&snapshot.observable_actions)
    .bind(&input.observable_actions_sha256)
    .bind(&snapshot.frozen_contract)
    .bind(&input.frozen_contract_sha256)
    .bind(&input.completion_claim)
    .bind(&input.completion_claim_sha256)
    .bind(&input.bundle_sha256)
    .execute(&mut *tx)
    .await?;
    let replayed = inserted.rows_affected() == 0;
    if replayed {
        let persisted = sqlx::query_as::<_, FrozenTargetIntelReviewReplayRow>(
            r#"SELECT operation_id, organization_id, stage_execution_id,
                      stage_run_unit_id, scope_snapshot_id, team_plan_id,
                      goal_epoch_id, goal_epoch, review_generation, round,
                      controller_work_item_id, controller_worker_run_id,
                      controller_message_chain_id, operation_contract_sha256,
                      material_revision_vector, material_state_sha256,
                      material_actions_sha256, durable_state,
                      durable_state_sha256, observable_actions,
                      observable_actions_sha256, frozen_contract,
                      frozen_contract_sha256, completion_claim,
                      completion_claim_sha256, bundle_sha256
                 FROM target_intel_goal_reviews
                WHERE id=$1
                FOR UPDATE"#,
        )
        .bind(input.review_id)
        .fetch_one(&mut *tx)
        .await?;
        let expected = FrozenTargetIntelReviewReplayRow {
            operation_id: snapshot.operation_id,
            organization_id: snapshot.organization_id,
            stage_execution_id: snapshot.stage_execution_id,
            stage_run_unit_id: snapshot.stage_run_unit_id,
            scope_snapshot_id: snapshot.scope_snapshot_id,
            team_plan_id: snapshot.team_plan_id,
            goal_epoch_id: snapshot.goal_epoch_id,
            goal_epoch: snapshot.goal_epoch,
            review_generation: snapshot.review_generation,
            round: snapshot.round,
            controller_work_item_id: snapshot.controller_work_item_id,
            controller_worker_run_id: Some(snapshot.controller_worker_run_id),
            controller_message_chain_id: snapshot.controller_message_chain_id,
            reviewer_work_item_id,
            operation_contract_sha256: snapshot.operation_contract_sha256.clone(),
            material_revision_vector: input.material_revision_vector.clone(),
            material_state_sha256: input.material_state_sha256.clone(),
            material_actions_sha256: input.material_actions_sha256.clone(),
            durable_state: snapshot.durable_state.clone(),
            durable_state_sha256: input.durable_state_sha256.clone(),
            observable_actions: snapshot.observable_actions.clone(),
            observable_actions_sha256: input.observable_actions_sha256.clone(),
            frozen_contract: snapshot.frozen_contract.clone(),
            frozen_contract_sha256: input.frozen_contract_sha256.clone(),
            completion_claim: input.completion_claim.clone(),
            completion_claim_sha256: input.completion_claim_sha256.clone(),
            bundle_sha256: input.bundle_sha256.clone(),
        };
        if persisted != expected {
            bail!("TARGET_INTEL_REVIEW_FREEZE_REPLAY_MISMATCH");
        }
    }
    if snapshot.runtime_mode == "observe_shadow" {
        sqlx::query(
            r#"INSERT INTO target_intel_goal_review_jobs (
                   id, review_id, mode, status
               ) VALUES ($1,$2,'observe_shadow','queued')
               ON CONFLICT (review_id) DO NOTHING"#,
        )
        .bind(Uuid::new_v4())
        .bind(input.review_id)
        .execute(&mut *tx)
        .await?;
    } else {
        let applied = sqlx::query(
            r#"UPDATE target_intel_goal_review_freeze_authorities
                  SET status='applied',row_version=row_version+1,applied_at=NOW()
                WHERE review_id=$1 AND status='building' AND row_version=0"#,
        )
        .bind(input.review_id)
        .execute(&mut *tx)
        .await?;
        if applied.rows_affected() != 1 {
            bail!("TARGET_INTEL_REVIEW_FREEZE_AUTHORITY_APPLY_FAILED");
        }
    }
    tx.commit().await?;
    Ok(InsertedFrozenTargetIntelReview {
        reviewer_work_item_id,
        replayed: false,
    })
}

pub(super) fn deterministic_child_id(review_id: Uuid, domain: &[u8]) -> Uuid {
    let digest = Sha256::new()
        .chain_update(b"target-intel-review:v1:")
        .chain_update(domain)
        .chain_update(review_id.as_bytes())
        .finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn canonical_sha256(value: &Value) -> String {
    fn write(value: &Value, output: &mut Vec<u8>) {
        match value {
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                output.extend(serde_json::to_vec(value).unwrap_or_default());
            }
            Value::Array(values) => {
                output.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    write(value, output);
                }
                output.push(b']');
            }
            Value::Object(map) => {
                output.push(b'{');
                let mut keys = map.keys().collect::<Vec<_>>();
                keys.sort();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    output.extend(serde_json::to_vec(key).unwrap_or_default());
                    output.push(b':');
                    write(&map[key], output);
                }
                output.push(b'}');
            }
        }
    }
    let mut bytes = Vec::new();
    write(value, &mut bytes);
    sha256_prefixed(&bytes)
}

fn recompute_review_bundle_sha256(input: &InsertFrozenTargetIntelReview) -> Result<String> {
    let snapshot = &input.snapshot;
    let controller_message_chain_id = snapshot
        .controller_message_chain_id
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_CONTROLLER_CHAIN_MISSING"))?;
    if snapshot.round <= 0 || snapshot.review_generation <= 0 || snapshot.state_revision < 0 {
        bail!("TARGET_INTEL_REVIEW_BUNDLE_IDENTITY_INVALID");
    }
    Ok(canonical_sha256(&serde_json::json!([
        {
            "review_id": input.review_id,
            "operation_id": snapshot.operation_id,
            "stage_execution_id": snapshot.stage_execution_id,
            "stage_run_unit_id": snapshot.stage_run_unit_id,
            "organization_id": snapshot.organization_id,
            "team_plan_id": snapshot.team_plan_id,
            "controller_work_item_id": snapshot.controller_work_item_id,
            "controller_worker_run_id": snapshot.controller_worker_run_id,
            "controller_message_chain_id": controller_message_chain_id,
            "goal_epoch": snapshot.goal_epoch,
            "review_generation": snapshot.review_generation,
            "round": snapshot.round,
            "state_revision": snapshot.state_revision,
        },
        [
            {
                "kind": "durable_state",
                "payload": snapshot.durable_state,
                "sha256": input.durable_state_sha256,
            },
            {
                "kind": "observable_actions",
                "payload": snapshot.observable_actions,
                "sha256": input.observable_actions_sha256,
            },
            {
                "kind": "frozen_contract",
                "payload": snapshot.frozen_contract,
                "sha256": input.frozen_contract_sha256,
            },
            {
                "kind": "completion_claim",
                "payload": input.completion_claim,
                "sha256": input.completion_claim_sha256,
            }
        ]
    ])))
}

/// Recompute the semantic identity of one reviewer finding from bounded host
/// fields. The reviewer-supplied fingerprint is never trusted as authority.
pub fn compute_finding_fingerprint(finding: &Value) -> Result<String> {
    let materiality = finding
        .get("materiality")
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "critical" | "major" | "minor" | "advisory"))
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_FINDING_SHAPE_INVALID"))?;
    let reason = finding
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_FINDING_SHAPE_INVALID"))?;
    let mut subject_refs = finding
        .get("subject_refs")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_FINDING_SHAPE_INVALID"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_FINDING_SHAPE_INVALID"))
        })
        .collect::<Result<Vec<_>>>()?;
    subject_refs.sort();
    subject_refs.dedup();
    let optional = |name: &str| -> Result<Option<String>> {
        match finding.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) if !value.trim().is_empty() => {
                Ok(Some(value.trim().to_string()))
            }
            _ => bail!("TARGET_INTEL_REVIEW_FINDING_SHAPE_INVALID"),
        }
    };
    Ok(canonical_sha256(&serde_json::json!({
        "materiality": materiality,
        "subject_refs": subject_refs,
        "reason": reason,
        "action_kind": optional("action_kind")?,
        "capability_ref": optional("capability_ref")?,
        "close_condition": optional("close_condition")?,
    })))
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct TargetIntelGoalFinalizerSnapshot {
    pub review_id: Uuid,
    pub operation_contract_sha256: String,
    pub review_bundle_sha256: String,
    pub verdict_sha256: String,
    pub operation_contract_valid: bool,
    pub review_is_fresh_pass: bool,
    pub all_four_sections_read: bool,
    pub material_revision_matches: bool,
    pub active_authoritative_workers: i64,
    pub active_authoritative_tools: i64,
    pub current_run_terminal_receipt_count: i64,
    pub valid_evidence_artifact_closure_count: i64,
    pub pending_or_retryable_frontier_count: i64,
    pub unwaived_blocked_or_unsupported_count: i64,
    pub unresolved_material_contradiction_count: i64,
    pub open_material_finding_count: i64,
    pub unauthorized_scope_promotion_count: i64,
    pub confirmed_company_identity_count: i64,
    pub structured_journal_entry_count: i64,
    pub completion_checkpoint_count: i64,
    pub unassessed_observation_count: i64,
    pub invalid_promotion_count: i64,
    pub orphan_formal_target_count: i64,
    pub needs_human_count: i64,
}

impl TargetIntelGoalFinalizerSnapshot {
    /// Mirror the I/O-free kit evaluator at the final write boundary. Keeping
    /// this fail-closed predicate beside the locked SQL material prevents an
    /// authorization/read transaction from drifting before StageTeam PASS.
    pub fn pass_block_code(&self) -> Option<&'static str> {
        if self.review_id.is_nil()
            || !self.operation_contract_valid
            || !is_prefixed_sha256(&self.operation_contract_sha256)
            || !is_prefixed_sha256(&self.review_bundle_sha256)
            || !is_prefixed_sha256(&self.verdict_sha256)
        {
            return Some("INTEL_GOAL_OPERATION_CONTRACT_INVALID");
        }
        if !self.review_is_fresh_pass || !self.all_four_sections_read {
            return Some("INTEL_GOAL_REVIEW_NOT_FRESH_PASS");
        }
        if !self.material_revision_matches {
            return Some("INTEL_GOAL_MATERIAL_DRIFT");
        }
        if self.active_authoritative_workers != 0 || self.active_authoritative_tools != 0 {
            return Some("INTEL_GOAL_ACTIVE_WORK_REMAINS");
        }
        if self.current_run_terminal_receipt_count <= 0
            || self.valid_evidence_artifact_closure_count != self.current_run_terminal_receipt_count
        {
            return Some("INTEL_GOAL_NON_VACUITY_FAILED");
        }
        if self.pending_or_retryable_frontier_count != 0 {
            return Some("INTEL_GOAL_FRONTIER_OPEN");
        }
        if self.unwaived_blocked_or_unsupported_count != 0 {
            return Some("INTEL_GOAL_CAPABILITY_GAP_UNRESOLVED");
        }
        if self.unresolved_material_contradiction_count != 0
            || self.open_material_finding_count != 0
            || self.needs_human_count != 0
        {
            return Some("INTEL_GOAL_MATERIAL_REVIEW_OPEN");
        }
        if self.unauthorized_scope_promotion_count != 0 {
            return Some("INTEL_GOAL_SCOPE_PROMOTION_VIOLATION");
        }
        if self.confirmed_company_identity_count != 1 {
            return Some("INTEL_GOAL_COMPANY_IDENTITY_MISSING");
        }
        if self.structured_journal_entry_count <= 0 || self.completion_checkpoint_count != 1 {
            return Some("INTEL_GOAL_WORK_MEMORY_INCOMPLETE");
        }
        if self.unassessed_observation_count != 0 {
            return Some("INTEL_GOAL_ATTRIBUTION_INCOMPLETE");
        }
        if self.invalid_promotion_count != 0 || self.orphan_formal_target_count != 0 {
            return Some("INTEL_GOAL_PROMOTION_CLOSURE_INVALID");
        }
        None
    }
}

fn is_prefixed_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

pub async fn load_finalizer_snapshot(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id: Uuid,
    review_id: Uuid,
    expected_bundle_sha256: &str,
    expected_verdict_sha256: &str,
    expected_operation_contract_sha256: &str,
    expected_review_row_version: i64,
) -> Result<TargetIntelGoalFinalizerSnapshot> {
    let mut tx = pool.begin().await?;
    let snapshot = load_finalizer_snapshot_with_connection(
        &mut tx,
        operation_id,
        organization_id,
        review_id,
        expected_bundle_sha256,
        expected_verdict_sha256,
        expected_operation_contract_sha256,
        expected_review_row_version,
    )
    .await?;
    tx.commit().await?;
    Ok(snapshot)
}

/// Caller-owned transaction variant used by the compound Intel + StageTeam
/// PASS seam. The review, operation contract and material revision rows remain
/// locked until the ordinary Unit final seal has landed.
pub async fn load_finalizer_snapshot_with_connection(
    connection: &mut PgConnection,
    operation_id: Uuid,
    organization_id: Uuid,
    review_id: Uuid,
    expected_bundle_sha256: &str,
    expected_verdict_sha256: &str,
    expected_operation_contract_sha256: &str,
    expected_review_row_version: i64,
) -> Result<TargetIntelGoalFinalizerSnapshot> {
    let snapshot = sqlx::query_as::<_, TargetIntelGoalFinalizerSnapshot>(
        r#"SELECT r.id AS review_id,
                  c.goal_contract_sha256 AS operation_contract_sha256,
                  r.bundle_sha256 AS review_bundle_sha256,
                  COALESCE(r.verdict_sha256, '') AS verdict_sha256,
                  (
                      c.runtime_mode='intel_goal_v1'
                      AND c.completion_authority='intel_goal_v1'
                      AND c.goal_contract_sha256=$6
                      AND r.operation_contract_sha256=c.goal_contract_sha256
                  ) AS operation_contract_valid,
                  (
                      r.status='pass'
                      AND r.bundle_sha256=$4
                      AND r.verdict_sha256=$5
                      AND NOT EXISTS (
                          SELECT 1 FROM target_intel_goal_reviews newer
                           WHERE newer.operation_id=r.operation_id
                             AND newer.organization_id=r.organization_id
                             AND newer.review_generation > r.review_generation
                      )
                  ) AS review_is_fresh_pass,
                  ((SELECT COUNT(*) FROM target_intel_goal_review_section_reads reads
                     WHERE reads.review_id=r.id)=4) AS all_four_sections_read,
                  (
                      COALESCE((r.material_revision_vector ->> 'state_revision')::bigint, -1)=m.state_revision
                      AND COALESCE((r.material_revision_vector ->> 'action_revision')::bigint, -1)=m.action_revision
                      AND COALESCE((r.material_revision_vector ->> 'evidence_high_water')::bigint, -1)
                          = GREATEST(m.evidence_high_water, COALESCE((
                              SELECT MAX(a.id) FROM audit_log a
                               WHERE a.audit_role='evidence'
                                 AND a.run_id=r.operation_id
                                 AND a.detail ->> 'organization_id'=r.organization_id::text
                                 AND NOT (
                                     a.action='stage_final_seal_attested'
                                     AND a.source='runtime_memory_final_seal'
                                     AND a.tool_name='runtime_memory_final_seal_attestation'
                                     AND a.detail ->> 'kind'='stage_final_seal_attestation'
                                 )
                          ), 0))
                      AND COALESCE((r.material_revision_vector ->> 'tool_high_water')::bigint, -1)
                          = GREATEST(m.tool_high_water, (
                              SELECT COUNT(*) FROM tool_calls t
                               WHERE t.operation_id=r.operation_id
                                 AND t.organization_id=r.organization_id
                                 AND t.stage_run_unit_id=r.stage_run_unit_id
                                 AND t.name NOT IN (
                                     'stage_team_request_intel_review',
                                     'target_intel_read_review_section',
                                     'target_intel_record_review_verdict',
                                     'target_intel_finalize_goal_pass'
                                 )
                                 AND NOT EXISTS (
                                     SELECT 1
                                       FROM stage_worker_runs review_worker
                                       JOIN stage_work_items review_item
                                         ON review_item.id=review_worker.work_item_id
                                      WHERE review_worker.id=t.worker_run_id
                                        AND review_worker.stage_run_unit_id=t.stage_run_unit_id
                                        AND review_item.stage_run_unit_id=t.stage_run_unit_id
                                        AND review_item.execution_profile='read_only_reviewer'
                                 )
                          ))
                      AND COALESCE(jsonb_array_length(r.observable_actions -> 'semantic_receipts'), -1)
                          = (SELECT COUNT(*) FROM audit_log a
                              WHERE a.details='intel.semantic_pivot_receipt.v1'
                                AND a.source IN ('target_intel_goal_shadow','target_intel_goal')
                                AND a.detail ->> 'operation_id'=r.operation_id::text
                                AND a.detail ->> 'organization_id'=r.organization_id::text)
                      AND COALESCE(jsonb_array_length(r.observable_actions -> 'query_receipts'), -1)
                          = (SELECT COUNT(*) FROM audit_log a
                              JOIN tool_calls tool
                                ON tool.id=CASE
                                    WHEN a.detail ->> 'producer_tool_call_id'
                                         ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                                    THEN (a.detail ->> 'producer_tool_call_id')::uuid
                                    ELSE NULL
                                END
                               AND tool.operation_id=r.operation_id
                               AND tool.organization_id=r.organization_id
                               AND tool.stage_run_unit_id=r.stage_run_unit_id
                               AND tool.worker_run_id::text=a.detail ->> 'producer_worker_run_id'
                               AND tool.name='recon_search_intel'
                               AND tool.status='finished'
                              WHERE a.details='intel.semantic_query_receipt.v1'
                                AND a.audit_role='evidence'
                                AND a.run_id=r.operation_id
                                AND a.detail ->> 'organization_id'=r.organization_id::text)
                      AND COALESCE(jsonb_array_length(r.observable_actions -> 'tool_calls'), -1)
                          = (SELECT COUNT(*) FROM tool_calls t
                              WHERE t.operation_id=r.operation_id
                                AND t.organization_id=r.organization_id
                                AND t.stage_run_unit_id=r.stage_run_unit_id
                                AND t.name NOT IN (
                                    'stage_team_request_intel_review',
                                    'target_intel_read_review_section',
                                    'target_intel_record_review_verdict',
                                    'target_intel_finalize_goal_pass'
                                )
                                AND NOT EXISTS (
                                    SELECT 1
                                      FROM stage_worker_runs review_worker
                                      JOIN stage_work_items review_item
                                        ON review_item.id=review_worker.work_item_id
                                     WHERE review_worker.id=t.worker_run_id
                                       AND review_worker.stage_run_unit_id=t.stage_run_unit_id
                                       AND review_item.stage_run_unit_id=t.stage_run_unit_id
                                       AND review_item.execution_profile='read_only_reviewer'
                                ))
                      AND COALESCE(jsonb_array_length(r.durable_state -> 'work_journal'), -1)
                          = (SELECT COUNT(*) FROM target_intel_goal_work_journal_entries journal
                              WHERE journal.operation_id=r.operation_id
                                AND journal.organization_id=r.organization_id
                                AND journal.team_plan_id=r.team_plan_id)
                      AND COALESCE(jsonb_array_length(r.durable_state -> 'asset_observations'), -1)
                          = (SELECT COUNT(*) FROM target_intel_asset_observations observation
                              WHERE observation.operation_id=r.operation_id
                                AND observation.organization_id=r.organization_id
                                AND observation.team_plan_id=r.team_plan_id)
                      AND COALESCE(jsonb_array_length(r.durable_state -> 'formal_assets'), -1)
                          = (SELECT COUNT(*) FROM target_intel_asset_observations observation
                              WHERE observation.operation_id=r.operation_id
                                AND observation.organization_id=r.organization_id
                                AND observation.team_plan_id=r.team_plan_id
                                AND observation.promotion_target_id IS NOT NULL)
                      AND COALESCE(jsonb_array_length(r.observable_actions -> 'promotion_events'), -1)
                          = (SELECT COUNT(*) FROM target_intel_asset_observation_events event
                              WHERE event.operation_id=r.operation_id
                                AND event.organization_id=r.organization_id
                                AND event.event_kind='promotion')
                  ) AS material_revision_matches,
                  (SELECT COUNT(*) FROM stage_worker_runs worker
                    WHERE worker.stage_run_unit_id=r.stage_run_unit_id
                      AND worker.id IS DISTINCT FROM r.reviewer_worker_run_id
                      AND worker.id IS DISTINCT FROM r.controller_worker_run_id
                      AND worker.status IN ('queued','running','waiting_background','gate_blocked','recovery_required'))
                      AS active_authoritative_workers,
                  (SELECT COUNT(*) FROM stage_worker_runs worker
                    WHERE worker.stage_run_unit_id=r.stage_run_unit_id
                      AND worker.id IS DISTINCT FROM r.reviewer_worker_run_id
                      AND worker.id IS DISTINCT FROM r.controller_worker_run_id
                      AND worker.active_tool_call_id IS NOT NULL)
                      AS active_authoritative_tools,
                  ((SELECT COUNT(*) FROM audit_log a
                     WHERE a.details='intel.semantic_pivot_receipt.v1'
                       AND a.source IN ('target_intel_goal_shadow','target_intel_goal')
                       AND a.status IN ('succeeded','empty','blocked','unsupported')
                       AND a.detail ->> 'operation_id'=r.operation_id::text
                       AND a.detail ->> 'organization_id'=r.organization_id::text)
                   +
                   (SELECT COUNT(*) FROM audit_log query_receipt
                     WHERE query_receipt.details='intel.semantic_query_receipt.v1'
                       AND query_receipt.audit_role='evidence'
                       AND query_receipt.evidence_outcome='checked_empty'
                       AND query_receipt.run_id=r.operation_id
                       AND query_receipt.detail ->> 'operation_id'=r.operation_id::text
                       AND query_receipt.detail ->> 'organization_id'=r.organization_id::text))
                      AS current_run_terminal_receipt_count,
                  ((SELECT COUNT(*) FROM audit_log a
                     JOIN target_intel_semantic_artifacts artifact
                       ON artifact.artifact_ref=a.detail ->> 'artifact_ref'
                      AND artifact.artifact_sha256=a.detail ->> 'artifact_sha256'
                      AND artifact.operation_id=r.operation_id
                      AND artifact.organization_id=r.organization_id
                      AND artifact.session_id::text=a.session_id
                     JOIN audit_log evidence
                       ON evidence.id=CASE
                           WHEN a.detail ->> 'evidence_ref' ~ '^audit:[0-9]+$'
                           THEN substring(a.detail ->> 'evidence_ref' FROM 7)::bigint
                           ELSE NULL
                       END
                      AND evidence.audit_role='evidence'
                      AND evidence.session_id=a.session_id
                      AND evidence.run_id=r.operation_id
                      AND evidence.detail ->> 'organization_id'=r.organization_id::text
                     JOIN target_intel_asset_observations observation
                       ON observation.semantic_receipt_audit_id=a.id
                      AND observation.operation_id=r.operation_id
                      AND observation.organization_id=r.organization_id
                      AND observation.team_plan_id=r.team_plan_id
                      AND observation.evidence_id=evidence.id
                      AND observation.artifact_ref=artifact.artifact_ref
                      AND observation.artifact_sha256=artifact.artifact_sha256
                     WHERE a.details='intel.semantic_pivot_receipt.v1'
                       AND a.source IN ('target_intel_goal_shadow','target_intel_goal')
                       AND a.detail ->> 'operation_id'=r.operation_id::text
                       AND a.detail ->> 'organization_id'=r.organization_id::text)
                   +
                   (SELECT COUNT(*) FROM audit_log query_receipt
                     JOIN tool_calls producer
                       ON producer.id=CASE
                           WHEN query_receipt.detail ->> 'producer_tool_call_id'
                                ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                           THEN (query_receipt.detail ->> 'producer_tool_call_id')::uuid
                           ELSE NULL
                       END
                      AND producer.operation_id=r.operation_id
                      AND producer.organization_id=r.organization_id
                      AND producer.stage_run_unit_id=r.stage_run_unit_id
                      AND producer.worker_run_id=r.controller_worker_run_id
                      AND producer.worker_run_id::text=query_receipt.detail ->> 'producer_worker_run_id'
                      AND producer.session_id::text=query_receipt.session_id
                      AND producer.name='recon_search_intel'
                      AND producer.status='finished'
                     JOIN target_intel_goal_epochs receipt_epoch
                       ON receipt_epoch.id=CASE
                           WHEN query_receipt.detail ->> 'goal_epoch_id'
                                ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                           THEN (query_receipt.detail ->> 'goal_epoch_id')::uuid
                           ELSE NULL
                       END
                      AND receipt_epoch.operation_id=r.operation_id
                      AND receipt_epoch.organization_id=r.organization_id
                      AND receipt_epoch.team_plan_id=r.team_plan_id
                     WHERE query_receipt.details='intel.semantic_query_receipt.v1'
                       AND query_receipt.audit_role='evidence'
                       AND query_receipt.evidence_outcome='checked_empty'
                       AND query_receipt.run_id=r.operation_id
                       AND query_receipt.detail ->> 'kind'='target_intel.semantic_query'
                       AND query_receipt.detail ->> 'operation_id'=r.operation_id::text
                       AND query_receipt.detail ->> 'organization_id'=r.organization_id::text
                       AND query_receipt.detail ->> 'provider_run_id'
                           ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                       AND query_receipt.detail ->> 'pivot_kind' IN (
                           'company_name','brand','domain','hostname','ip','cidr','asn',
                           'certificate','icp','email_domain','github_org','repository','app_id'
                       )
                       AND query_receipt.detail ->> 'pivot_value_sha256' ~ '^[0-9a-f]{64}$'
                       AND jsonb_typeof(query_receipt.detail -> 'counts')='object'
                       AND query_receipt.detail #>> '{counts,candidate_targets}'='0'
                       AND query_receipt.detail #>> '{counts,profile_fields}'='0'
                       AND jsonb_typeof(query_receipt.detail -> 'provider_status') IN ('array','object')
                       AND jsonb_typeof(query_receipt.detail -> 'technique_status') IN ('array','object')))
                      AS valid_evidence_artifact_closure_count,
                  (SELECT COUNT(*) FROM target_intel_goal_frontier_v2 frontier
                    WHERE frontier.operation_id=r.operation_id
                      AND frontier.organization_id=r.organization_id
                      AND frontier.materiality='material'
                      AND frontier.status IN ('pending','in_progress'))
                      AS pending_or_retryable_frontier_count,
                  (SELECT COUNT(*) FROM target_intel_goal_frontier_v2 frontier
                    WHERE frontier.operation_id=r.operation_id
                      AND frontier.organization_id=r.organization_id
                      AND frontier.materiality='material'
                      AND frontier.status IN ('blocked','unsupported')
                      AND NOT EXISTS (
                          SELECT 1 FROM target_intel_goal_frontier_waivers waiver
                           WHERE waiver.frontier_id=frontier.id
                             AND waiver.operation_id=frontier.operation_id
                             AND waiver.organization_id=frontier.organization_id
                             AND waiver.expected_frontier_row_version=frontier.row_version
                      ))
                      AS unwaived_blocked_or_unsupported_count,
                  (SELECT COUNT(*)
                     FROM target_intel_goal_reviews prior_review
                     JOIN target_intel_goal_review_findings finding
                       ON finding.review_id=prior_review.id
                    WHERE prior_review.operation_id=r.operation_id
                      AND prior_review.organization_id=r.organization_id
                      AND prior_review.review_generation < r.review_generation
                      AND finding.materiality IN ('critical','major')
                      AND NOT EXISTS (
                          SELECT 1
                            FROM target_intel_goal_review_finding_resolutions resolution
                           WHERE resolution.review_id=r.id
                             AND resolution.finding_id=finding.id
                             AND resolution.disposition='resolved'
                      ))
                      AS unresolved_material_contradiction_count,
                  (SELECT COUNT(*) FROM target_intel_goal_review_findings finding
                    WHERE finding.review_id=r.id
                      AND finding.materiality IN ('critical','major'))
                      AS open_material_finding_count,
                  (SELECT COALESCE(SUM(jsonb_array_length(a.detail -> 'unauthorized_promotion_refs')), 0)
                     FROM audit_log a
                    WHERE a.details='intel.semantic_pivot_receipt.v1'
                      AND a.source IN ('target_intel_goal_shadow','target_intel_goal')
                      AND jsonb_typeof(a.detail -> 'unauthorized_promotion_refs')='array'
                      AND a.detail ->> 'operation_id'=r.operation_id::text
                      AND a.detail ->> 'organization_id'=r.organization_id::text)
                      AS unauthorized_scope_promotion_count,
                  (SELECT COUNT(*)
                     FROM target_intel_goal_company_identity_bindings binding
                     JOIN scoping_company_identity_receipts identity
                       ON identity.id=binding.company_identity_receipt_id
                      AND identity.operation_id=binding.operation_id
                      AND identity.organization_id=binding.organization_id
                    WHERE binding.operation_id=r.operation_id
                      AND binding.organization_id=r.organization_id
                      AND identity.resolution_status='confirmed'
                      AND identity.identity_sha256=binding.company_identity_sha256
                      AND identity.scope_policy_sha256=binding.scope_policy_sha256)
                      AS confirmed_company_identity_count,
                  (SELECT COUNT(*) FROM target_intel_goal_work_journal_entries journal
                    WHERE journal.operation_id=r.operation_id
                      AND journal.organization_id=r.organization_id
                      AND journal.team_plan_id=r.team_plan_id)
                      AS structured_journal_entry_count,
                  (SELECT COUNT(*) FROM target_intel_goal_work_journal_entries journal
                    WHERE journal.operation_id=r.operation_id
                      AND journal.organization_id=r.organization_id
                      AND journal.team_plan_id=r.team_plan_id
                      AND journal.entry_kind='completion_checkpoint')
                      AS completion_checkpoint_count,
                  (SELECT COUNT(*) FROM target_intel_asset_observations observation
                    WHERE observation.operation_id=r.operation_id
                      AND observation.organization_id=r.organization_id
                      AND observation.team_plan_id=r.team_plan_id
                      AND observation.attribution_disposition='unassessed')
                      AS unassessed_observation_count,
                  (SELECT COUNT(*)
                     FROM target_intel_asset_observations observation
                     LEFT JOIN targets target ON target.id=observation.promotion_target_id
                    WHERE observation.operation_id=r.operation_id
                      AND observation.organization_id=r.organization_id
                      AND observation.team_plan_id=r.team_plan_id
                      AND observation.promotion_target_id IS NOT NULL
                      AND (
                          observation.attribution_disposition<>'owned'
                          OR observation.reachability_state<>'reachable'
                          OR observation.reachability_valid_until<=observation.promoted_at
                          OR target.id IS NULL
                          OR target.organization_id IS DISTINCT FROM r.organization_id
                          OR target.scope::text<>'in'
                          OR target.source<>'target_intel_goal'
                          OR target.liveness_state<>'alive'
                          OR target.liveness_checked_at IS NULL
                          OR target.liveness_checked_at<observation.reachability_checked_at
                      )) AS invalid_promotion_count,
                  (SELECT COUNT(*) FROM targets target
                    WHERE target.organization_id=r.organization_id
                      AND target.source='target_intel_goal'
                      AND NOT EXISTS (
                          SELECT 1 FROM target_intel_asset_observations observation
                           WHERE observation.operation_id=r.operation_id
                             AND observation.organization_id=r.organization_id
                             AND observation.promotion_target_id=target.id
                      )) AS orphan_formal_target_count,
                  ((SELECT COUNT(*) FROM target_intel_goal_holds hold
                     WHERE hold.operation_id=r.operation_id
                       AND hold.organization_id=r.organization_id
                       AND hold.status='open')
                   +
                   (SELECT COUNT(*) FROM target_intel_goal_frontier_v2 frontier
                     WHERE frontier.operation_id=r.operation_id
                       AND frontier.organization_id=r.organization_id
                       AND frontier.materiality='material'
                       AND frontier.status='needs_human')) AS needs_human_count
             FROM target_intel_goal_reviews r
             JOIN target_intel_goal_operation_contracts c ON c.operation_id=r.operation_id
             JOIN target_intel_goal_material_revisions m
               ON m.operation_id=r.operation_id AND m.organization_id=r.organization_id
            WHERE r.id=$3 AND r.operation_id=$1 AND r.organization_id=$2
              AND r.row_version=$7
            FOR UPDATE OF r,c,m"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(review_id)
    .bind(expected_bundle_sha256)
    .bind(expected_verdict_sha256)
    .bind(expected_operation_contract_sha256)
    .bind(expected_review_row_version)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_FINALIZER_MATERIAL_MISSING"))?;
    Ok(snapshot)
}

#[derive(Debug, Clone, PartialEq)]
pub struct TargetIntelReviewSectionRow {
    pub review_id: Uuid,
    pub review_row_version: i64,
    pub ordinal: i16,
    pub kind: String,
    pub sha256: String,
    pub payload: Value,
    pub replayed: bool,
}

pub async fn load_inherited_material_finding_ids(
    pool: &PgPool,
    review_id: Uuid,
) -> Result<Vec<Uuid>> {
    sqlx::query_scalar(
        r#"SELECT finding.id
             FROM target_intel_goal_reviews current_review
             JOIN target_intel_goal_reviews prior_review
               ON prior_review.operation_id=current_review.operation_id
              AND prior_review.organization_id=current_review.organization_id
              AND prior_review.review_generation < current_review.review_generation
             JOIN target_intel_goal_review_findings finding
               ON finding.review_id=prior_review.id
            WHERE current_review.id=$1
              AND finding.materiality IN ('critical','major')
              AND NOT EXISTS (
                  SELECT 1
                    FROM target_intel_goal_review_finding_resolutions resolution
                    JOIN target_intel_goal_reviews resolution_review
                      ON resolution_review.id=resolution.review_id
                   WHERE resolution.finding_id=finding.id
                     AND resolution.disposition='resolved'
                     AND resolution_review.review_generation < current_review.review_generation
              )
            ORDER BY prior_review.review_generation, finding.id"#,
    )
    .bind(review_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub async fn read_section(
    pool: &PgPool,
    review_id: Uuid,
    reviewer_worker_run_id: Uuid,
    expected_worker_attempt_epoch: i64,
    requested_kind: &str,
    expected_bundle_sha256: &str,
) -> Result<TargetIntelReviewSectionRow> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query_as::<
        _,
        (
            Option<Uuid>,
            String,
            String,
            i64,
            String,
            Value,
            String,
            Value,
            String,
            Value,
            String,
            Value,
        ),
    >(
        r#"SELECT reviewer_worker_run_id, status, bundle_sha256, row_version,
                  durable_state_sha256, durable_state,
                  observable_actions_sha256, observable_actions,
                  frozen_contract_sha256, frozen_contract,
                  completion_claim_sha256, completion_claim
             FROM target_intel_goal_reviews
            WHERE id = $1
              AND EXISTS (
                  SELECT 1 FROM stage_worker_runs worker
                   WHERE worker.id=target_intel_goal_reviews.reviewer_worker_run_id
                     AND worker.attempt_epoch=$2
                     AND worker.status IN ('running','waiting_background','gate_blocked')
              )
            FOR UPDATE"#,
    )
    .bind(review_id)
    .bind(expected_worker_attempt_epoch)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_MISSING"))?;
    if row.0 != Some(reviewer_worker_run_id)
        || !matches!(row.1.as_str(), "frozen" | "reviewing")
        || row.2 != expected_bundle_sha256
    {
        bail!("TARGET_INTEL_REVIEW_FOREIGN_OR_STALE_READER");
    }
    let definitions = [
        ("durable_state", row.4, row.5),
        ("observable_actions", row.6, row.7),
        ("frozen_contract", row.8, row.9),
        ("completion_claim", row.10, row.11),
    ];
    let requested = definitions
        .iter()
        .position(|(kind, _, _)| *kind == requested_kind)
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_SECTION_KIND_INVALID"))?;
    let existing = sqlx::query_as::<_, (i16, String, String)>(
        r#"SELECT section_ordinal, section_kind, section_sha256
             FROM target_intel_goal_review_section_reads
            WHERE review_id = $1
            ORDER BY section_ordinal"#,
    )
    .bind(review_id)
    .fetch_all(&mut *tx)
    .await?;
    if requested < existing.len() {
        let prior = &existing[requested];
        if prior.1 != requested_kind || prior.2 != definitions[requested].1 {
            bail!("TARGET_INTEL_REVIEW_SECTION_REPLAY_MISMATCH");
        }
        tx.commit().await?;
        return Ok(TargetIntelReviewSectionRow {
            review_id,
            review_row_version: row.3,
            ordinal: prior.0,
            kind: prior.1.clone(),
            sha256: prior.2.clone(),
            payload: definitions[requested].2.clone(),
            replayed: true,
        });
    }
    if requested != existing.len() {
        bail!("TARGET_INTEL_REVIEW_SECTION_OUT_OF_ORDER");
    }
    let ordinal = (requested + 1) as i16;
    sqlx::query(
        r#"INSERT INTO target_intel_goal_review_section_reads (
               review_id, reviewer_worker_run_id, section_ordinal,
               section_kind, section_sha256,operation_id,organization_id
           ) SELECT $1,$2,$3,$4,$5,operation_id,organization_id
               FROM target_intel_goal_reviews WHERE id=$1"#,
    )
    .bind(review_id)
    .bind(reviewer_worker_run_id)
    .bind(ordinal)
    .bind(requested_kind)
    .bind(&definitions[requested].1)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE target_intel_goal_reviews SET status = 'reviewing', row_version = row_version + 1 WHERE id = $1 AND status = 'frozen'",
    )
    .bind(review_id)
    .execute(&mut *tx)
    .await?;
    let review_row_version = if row.1 == "frozen" {
        row.3
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_ROW_VERSION_OVERFLOW"))?
    } else {
        row.3
    };
    tx.commit().await?;
    Ok(TargetIntelReviewSectionRow {
        review_id,
        review_row_version,
        ordinal,
        kind: requested_kind.to_string(),
        sha256: definitions[requested].1.clone(),
        payload: definitions[requested].2.clone(),
        replayed: false,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedTargetIntelReviewVerdict {
    pub replayed: bool,
    pub review_row_version: i64,
    pub hold_id: Option<Uuid>,
    pub effective_decision: String,
    pub successor_goal_epoch_id: Option<Uuid>,
}

#[derive(Debug, sqlx::FromRow)]
struct ReviewVerdictAuthorityRow {
    reviewer_worker_run_id: Option<Uuid>,
    status: String,
    row_version: i64,
    bundle_sha256: String,
    verdict: Option<Value>,
    verdict_sha256: Option<String>,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    team_plan_id: Uuid,
    goal_epoch_id: Uuid,
    goal_epoch: i64,
    review_generation: i64,
    round: i32,
    controller_work_item_id: Uuid,
    controller_worker_run_id: Option<Uuid>,
    controller_message_chain_id: Option<Uuid>,
    material_state_sha256: String,
    material_actions_sha256: String,
    runtime_mode: String,
    max_review_rounds: i32,
    review_fuel_remaining: i32,
    epoch_status: String,
    epoch_row_version: i64,
    plan_row_version: i64,
    item_row_version: i64,
    plan_dispatch_epoch: i64,
    plan_is_closed: bool,
    final_submitter_authority_valid: bool,
}

pub async fn record_terminal_verdict(
    pool: &PgPool,
    review_id: Uuid,
    reviewer_worker_run_id: Uuid,
    expected_worker_attempt_epoch: i64,
    expected_row_version: i64,
    expected_bundle_sha256: &str,
    decision: &str,
    verdict: &Value,
    verdict_sha256: &str,
) -> Result<RecordedTargetIntelReviewVerdict> {
    if !matches!(decision, "pass" | "rework" | "needs_human")
        || canonical_sha256(verdict) != verdict_sha256
        || verdict.get("schema").and_then(Value::as_str) != Some("intel_review.v1")
        || verdict
            .get("decision")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .as_deref()
            != Some(decision)
        || !verdict.get("findings").is_some_and(Value::is_array)
        || !verdict
            .get("inherited_dispositions")
            .is_some_and(Value::is_array)
        || !verdict.get("residuals").is_some_and(Value::is_array)
    {
        bail!("TARGET_INTEL_REVIEW_DECISION_INVALID");
    }
    let mut tx = pool.begin().await?;
    let review = sqlx::query_as::<_, ReviewVerdictAuthorityRow>(
        r#"SELECT review.reviewer_worker_run_id,review.status,review.row_version,
                  review.bundle_sha256,review.verdict,review.verdict_sha256,
                  review.operation_id,review.organization_id,
                  review.stage_execution_id,review.stage_run_unit_id,
                  review.scope_snapshot_id,review.team_plan_id,
                  review.goal_epoch_id,review.goal_epoch,
                  review.review_generation,review.round,
                  review.controller_work_item_id,review.controller_worker_run_id,
                  review.controller_message_chain_id,
                  review.material_state_sha256,review.material_actions_sha256,
                  contract.runtime_mode,contract.max_review_rounds,
                  epoch.review_fuel_remaining,epoch.status AS epoch_status,
                  epoch.row_version AS epoch_row_version,
                  plan.row_version AS plan_row_version,
                  item.row_version AS item_row_version,
                  plan.dispatch_epoch AS plan_dispatch_epoch,
                  plan.requests_closed_at IS NOT NULL AS plan_is_closed,
                  (
                      plan.final_submitter_worker_run_id IS NULL
                      OR (
                          plan.final_submitter_worker_run_id=review.controller_worker_run_id
                          AND EXISTS (
                              SELECT 1 FROM stage_deliverable_submissions submission
                               WHERE submission.operation_id=review.operation_id
                                 AND submission.stage_execution_id=review.stage_execution_id
                                 AND submission.stage_run_unit_id=review.stage_run_unit_id
                                 AND submission.organization_id=review.organization_id
                                 AND submission.worker_run_id=review.controller_worker_run_id
                                 AND submission.stage_kind='target_intel'
                          )
                      )
                  ) AS final_submitter_authority_valid
             FROM target_intel_goal_reviews review
             JOIN target_intel_goal_operation_contracts contract
               ON contract.operation_id=review.operation_id
             JOIN target_intel_goal_epochs epoch ON epoch.id=review.goal_epoch_id
             JOIN stage_team_plans plan ON plan.id=review.team_plan_id
             JOIN stage_work_items item ON item.id=review.controller_work_item_id
            WHERE review.id=$1
            FOR UPDATE OF review,epoch,plan,item"#,
    )
    .bind(review_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_MISSING"))?;
    if matches!(review.status.as_str(), "pass" | "rework" | "needs_human") {
        if review.reviewer_worker_run_id != Some(reviewer_worker_run_id)
            || review.bundle_sha256 != expected_bundle_sha256
            || review.verdict.as_ref() != Some(verdict)
            || review.verdict_sha256.as_deref() != Some(verdict_sha256)
        {
            bail!("TARGET_INTEL_REVIEW_VERDICT_REPLAY_MISMATCH");
        }
        let hold_id =
            sqlx::query_scalar("SELECT id FROM target_intel_goal_holds WHERE review_id=$1")
                .bind(review_id)
                .fetch_optional(&mut *tx)
                .await?;
        tx.commit().await?;
        let successor_goal_epoch_id = sqlx::query_scalar(
            "SELECT successor_goal_epoch_id FROM target_intel_goal_resume_authorities WHERE source_review_id=$1 AND status='applied'",
        )
        .bind(review_id)
        .fetch_optional(pool)
        .await?;
        return Ok(RecordedTargetIntelReviewVerdict {
            replayed: true,
            review_row_version: review.row_version,
            hold_id,
            effective_decision: review.status,
            successor_goal_epoch_id,
        });
    }
    if review.reviewer_worker_run_id != Some(reviewer_worker_run_id)
        || review.status != "reviewing"
        || review.row_version != expected_row_version
        || review.bundle_sha256 != expected_bundle_sha256
        || review.epoch_status != "sealed_for_review"
        || !review.plan_is_closed
        || !review.final_submitter_authority_valid
        || review.plan_dispatch_epoch != review.goal_epoch
    {
        bail!("TARGET_INTEL_REVIEW_VERDICT_CAS_FAILED");
    }
    let active_attempt: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
               SELECT 1 FROM stage_worker_runs worker
                WHERE worker.id=$1
                  AND worker.attempt_epoch=$2
                  AND worker.status IN ('running','waiting_background','gate_blocked')
           )"#,
    )
    .bind(reviewer_worker_run_id)
    .bind(expected_worker_attempt_epoch)
    .fetch_one(&mut *tx)
    .await?;
    if !active_attempt {
        bail!("TARGET_INTEL_REVIEWER_ATTEMPT_STALE");
    }
    let read_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM target_intel_goal_review_section_reads WHERE review_id = $1",
    )
    .bind(review_id)
    .fetch_one(&mut *tx)
    .await?;
    if read_count != 4 {
        bail!("TARGET_INTEL_REVIEW_SECTIONS_INCOMPLETE");
    }
    let findings = verdict["findings"]
        .as_array()
        .expect("validated review findings array");
    if findings.len() > 128 {
        bail!("TARGET_INTEL_REVIEW_FINDING_SHAPE_INVALID");
    }
    let mut finding_ids = BTreeSet::new();
    let mut fingerprints = BTreeSet::new();
    let mut material_fingerprints = Vec::new();
    let mut recommended_actions = Vec::new();
    for finding in findings {
        let finding_id = finding
            .get("finding_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .filter(|value| !value.is_nil())
            .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_FINDING_SHAPE_INVALID"))?;
        let supplied = finding
            .get("fingerprint")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_FINDING_SHAPE_INVALID"))?;
        let recomputed = compute_finding_fingerprint(finding)?;
        if supplied != recomputed
            || !finding_ids.insert(finding_id)
            || !fingerprints.insert(recomputed.clone())
        {
            bail!("TARGET_INTEL_REVIEW_FINDING_FINGERPRINT_MISMATCH");
        }
        if matches!(
            finding.get("materiality").and_then(Value::as_str),
            Some("critical" | "major")
        ) {
            material_fingerprints.push(recomputed.clone());
        }
        recommended_actions.push(serde_json::json!({
            "fingerprint": recomputed,
            "action_kind": finding.get("action_kind").cloned().unwrap_or(Value::Null),
            "capability_ref": finding.get("capability_ref").cloned().unwrap_or(Value::Null),
            "close_condition": finding.get("close_condition").cloned().unwrap_or(Value::Null),
        }));
    }
    material_fingerprints.sort();
    recommended_actions.sort_by(|left, right| {
        left["fingerprint"]
            .as_str()
            .cmp(&right["fingerprint"].as_str())
    });
    let finding_set_sha256 = canonical_sha256(&serde_json::json!(material_fingerprints.clone()));
    let recommended_actions_sha256 = canonical_sha256(&serde_json::json!(recommended_actions));

    let expected_inherited = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT finding.id
             FROM target_intel_goal_reviews prior
             JOIN target_intel_goal_review_findings finding ON finding.review_id=prior.id
            WHERE prior.operation_id=$1 AND prior.organization_id=$2
              AND prior.review_generation<$3
              AND finding.materiality IN ('critical','major')
              AND NOT EXISTS (
                  SELECT 1 FROM target_intel_goal_review_finding_resolutions resolution
                  JOIN target_intel_goal_reviews resolution_review
                    ON resolution_review.id=resolution.review_id
                 WHERE resolution.finding_id=finding.id
                   AND resolution.disposition='resolved'
                   AND resolution_review.review_generation<$3
              )"#,
    )
    .bind(review.operation_id)
    .bind(review.organization_id)
    .bind(review.review_generation)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect::<BTreeSet<_>>();
    let supplied_inherited = verdict["inherited_dispositions"]
        .as_array()
        .expect("validated inherited dispositions array")
        .iter()
        .map(|item| {
            item.get("finding_id")
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_INHERITED_FINDING_INVALID"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if supplied_inherited != expected_inherited
        || supplied_inherited.len()
            != verdict["inherited_dispositions"]
                .as_array()
                .map_or(0, Vec::len)
    {
        bail!("TARGET_INTEL_REVIEW_INHERITED_FINDING_NOT_DISPOSED");
    }
    let inherited_dispositions = verdict["inherited_dispositions"]
        .as_array()
        .expect("validated inherited dispositions array");
    let inherited_shape_valid = inherited_dispositions.iter().all(|item| {
        let disposition = item.get("disposition").and_then(Value::as_str);
        let refs = item.get("resolution_refs").and_then(Value::as_array);
        let reason = item
            .get("reason")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        matches!(disposition, Some("resolved" | "still_open" | "needs_human"))
            && refs.is_some()
            && reason.is_some()
            && (disposition != Some("resolved") || refs.is_some_and(|refs| !refs.is_empty()))
    });
    let finding_shape_valid = findings.iter().all(|finding| {
        finding
            .get("evidence_refs")
            .and_then(Value::as_array)
            .is_some()
            && finding
                .get("subject_refs")
                .and_then(Value::as_array)
                .is_some()
    });
    if !inherited_shape_valid || !finding_shape_valid {
        bail!("TARGET_INTEL_REVIEW_VERDICT_SHAPE_INVALID");
    }
    if decision == "pass"
        && (!material_fingerprints.is_empty()
            || inherited_dispositions
                .iter()
                .any(|item| item.get("disposition").and_then(Value::as_str) != Some("resolved"))
            || !matches!(verdict.get("human_requirement"), None | Some(Value::Null)))
    {
        bail!("TARGET_INTEL_REVIEW_PASS_HAS_OPEN_MATERIAL_FINDING");
    }
    if decision == "rework"
        && !findings.iter().any(|finding| {
            matches!(
                finding.get("materiality").and_then(Value::as_str),
                Some("critical" | "major")
            ) && finding
                .get("evidence_refs")
                .and_then(Value::as_array)
                .is_some_and(|refs| !refs.is_empty())
                && finding
                    .get("action_kind")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
                && finding
                    .get("close_condition")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
        })
    {
        bail!("TARGET_INTEL_REVIEW_REWORK_NOT_ACTIONABLE");
    }
    let ungrounded_evidence_refs: i64 = sqlx::query_scalar(
        r#"WITH cited(ref) AS (
               SELECT evidence_ref.ref
                 FROM jsonb_array_elements($2 -> 'findings') AS finding
                 CROSS JOIN LATERAL jsonb_array_elements_text(
                     finding -> 'evidence_refs'
                 ) AS evidence_ref(ref)
               UNION ALL
               SELECT resolution_ref.ref
                 FROM jsonb_array_elements($2 -> 'inherited_dispositions') AS disposition
                 CROSS JOIN LATERAL jsonb_array_elements_text(
                     disposition -> 'resolution_refs'
                 ) AS resolution_ref(ref)
                WHERE disposition ->> 'disposition'='resolved'
           )
           SELECT COUNT(*)
             FROM cited
             JOIN target_intel_goal_reviews review ON review.id=$1
            WHERE cited.ref !~ '^audit:[0-9]+$'
               OR NOT EXISTS (
                   SELECT 1 FROM audit_log evidence
                    WHERE evidence.id=CASE
                        WHEN cited.ref ~ '^audit:[0-9]+$'
                        THEN substring(cited.ref FROM 7)::bigint
                        ELSE NULL
                    END
                      AND evidence.audit_role='evidence'
                      AND evidence.run_id=review.operation_id
                      AND evidence.detail ->> 'organization_id'=review.organization_id::text
               )"#,
    )
    .bind(review_id)
    .bind(verdict)
    .fetch_one(&mut *tx)
    .await?;
    if ungrounded_evidence_refs != 0 {
        bail!("TARGET_INTEL_REVIEW_EVIDENCE_REF_UNGROUNDED");
    }
    let fixed_point: bool = if decision == "rework" {
        sqlx::query_scalar(
            r#"SELECT EXISTS (
                   SELECT 1 FROM target_intel_goal_reviews prior
                    WHERE prior.operation_id=$1 AND prior.organization_id=$2
                      AND prior.review_generation<$3
                      AND prior.finding_set_sha256=$4
                      AND prior.recommended_actions_sha256=$5
                      AND prior.material_revision_vector=(
                          SELECT current.material_revision_vector
                            FROM target_intel_goal_reviews current
                           WHERE current.id=$6
                      )
               )"#,
        )
        .bind(review.operation_id)
        .bind(review.organization_id)
        .bind(review.review_generation)
        .bind(&finding_set_sha256)
        .bind(&recommended_actions_sha256)
        .bind(review_id)
        .fetch_one(&mut *tx)
        .await?
    } else {
        false
    };
    let fuel_exhausted = decision == "rework"
        && (review.review_fuel_remaining <= 1 || review.round >= review.max_review_rounds);
    let (effective_decision, effective_reason, requirement_kind) =
        if review.runtime_mode == "observe_shadow" {
            (decision, None, None)
        } else if fixed_point {
            (
                "needs_human",
                Some("same_finding_without_material_delta"),
                Some("review_fixed_point"),
            )
        } else if fuel_exhausted {
            (
                "needs_human",
                Some("frozen_review_fuel_exhausted"),
                Some("review_fuel_exhausted"),
            )
        } else if decision == "needs_human" {
            let requirement = verdict
                .get("human_requirement")
                .and_then(Value::as_str)
                .filter(|value| {
                    matches!(
                        *value,
                        "credential"
                            | "scope_confirmation"
                            | "subject_confirmation"
                            | "provider_recovery"
                            | "review_fixed_point"
                    )
                })
                .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_HUMAN_REQUIREMENT_INVALID"))?;
            (
                "needs_human",
                Some("reviewer_requested_human_input"),
                Some(requirement),
            )
        } else {
            (decision, None, None)
        };
    sqlx::query(
        r#"INSERT INTO target_intel_goal_review_findings (
               id, review_id, fingerprint, materiality, subject_refs, reason,
               recommended_action, close_condition,operation_id,organization_id
           ) SELECT (item ->> 'finding_id')::uuid, $1,
                  item ->> 'fingerprint', item ->> 'materiality',
                  item -> 'subject_refs', item ->> 'reason',
                  jsonb_build_object(
                      'evidence_refs', item -> 'evidence_refs',
                      'action_kind', item -> 'action_kind',
                      'capability_ref', item -> 'capability_ref'
                  ),
                  item ->> 'close_condition',review.operation_id,review.organization_id
             FROM jsonb_array_elements($2 -> 'findings') AS item
             CROSS JOIN target_intel_goal_reviews review
            WHERE review.id=$1"#,
    )
    .bind(review_id)
    .bind(verdict)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO target_intel_goal_review_finding_resolutions (
               id,finding_id,review_id,disposition,resolution_refs,reason,
               operation_id,organization_id
           ) SELECT gen_random_uuid(),(item ->> 'finding_id')::uuid,$1,
                    item ->> 'disposition',item -> 'resolution_refs',item ->> 'reason',
                    review.operation_id,review.organization_id
               FROM jsonb_array_elements($2 -> 'inherited_dispositions') AS item
               CROSS JOIN target_intel_goal_reviews review
              WHERE review.id=$1"#,
    )
    .bind(review_id)
    .bind(verdict)
    .execute(&mut *tx)
    .await?;
    let updated = sqlx::query(
        r#"UPDATE target_intel_goal_reviews
              SET status=$1,verdict=$2,verdict_sha256=$3,
                  finding_set_sha256=$4,recommended_actions_sha256=$5,
                  effective_decision_reason=$6,terminal_at=NOW(),
                  row_version=row_version+1
            WHERE id=$7 AND reviewer_worker_run_id=$8 AND row_version=$9
              AND bundle_sha256=$10 AND status='reviewing'"#,
    )
    .bind(effective_decision)
    .bind(verdict)
    .bind(verdict_sha256)
    .bind(&finding_set_sha256)
    .bind(&recommended_actions_sha256)
    .bind(effective_reason)
    .bind(review_id)
    .bind(reviewer_worker_run_id)
    .bind(expected_row_version)
    .bind(expected_bundle_sha256)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() != 1 {
        bail!("TARGET_INTEL_REVIEW_VERDICT_CAS_FAILED");
    }
    let hold_id = if effective_decision == "needs_human" && review.runtime_mode != "observe_shadow"
    {
        let hold_id = deterministic_child_id(review_id, b"human-hold");
        let requirement_kind = requirement_kind
            .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_HUMAN_REQUIREMENT_INVALID"))?;
        sqlx::query(
            r#"INSERT INTO target_intel_goal_holds (
                   id, review_id, operation_id, organization_id,
                   requirement_kind, requirement_payload, status
               )
               SELECT $1, r.id, r.operation_id, r.organization_id,
                      $2,jsonb_build_object(
                          'review_id', r.id,
                          'verdict_sha256', $3::text,
                          'residuals', $4 -> 'residuals',
                          'accepted_fulfillment_kinds',CASE $2::text
                              WHEN 'credential' THEN '["credential_provided"]'::jsonb
                              WHEN 'scope_confirmation' THEN '["scope_confirmed"]'::jsonb
                              WHEN 'subject_confirmation' THEN '["subject_confirmed"]'::jsonb
                              WHEN 'provider_recovery' THEN '["provider_recovered"]'::jsonb
                              ELSE '["operator_override"]'::jsonb
                          END
                      ), 'open'
                 FROM target_intel_goal_reviews r
                WHERE r.id=$5"#,
        )
        .bind(hold_id)
        .bind(requirement_kind)
        .bind(verdict_sha256)
        .bind(verdict)
        .bind(review_id)
        .execute(&mut *tx)
        .await?;
        let held = sqlx::query(
            r#"UPDATE target_intel_goal_epochs
                  SET status='held',terminal_at=NOW(),row_version=row_version+1
                WHERE id=$1 AND status='sealed_for_review' AND row_version=$2"#,
        )
        .bind(review.goal_epoch_id)
        .bind(review.epoch_row_version)
        .execute(&mut *tx)
        .await?;
        if held.rows_affected() != 1 {
            bail!("TARGET_INTEL_REVIEW_HOLD_EPOCH_CAS_FAILED");
        }
        Some(hold_id)
    } else {
        None
    };
    let successor_goal_epoch_id = if effective_decision == "rework"
        && review.runtime_mode != "observe_shadow"
    {
        Some(
            apply_goal_resume(
                &mut tx,
                &review,
                review_id,
                None,
                None,
                "review_rework",
                &finding_set_sha256,
                &recommended_actions_sha256,
                serde_json::json!({
                    "kind":"target_intel_review_rework",
                    "review_id":review_id,
                    "finding_set_sha256":finding_set_sha256,
                        "recommended_actions_sha256":recommended_actions_sha256,
                        "findings":verdict.get("findings").cloned().unwrap_or_else(|| serde_json::json!([])),
                        "inherited_dispositions":verdict.get("inherited_dispositions").cloned().unwrap_or_else(|| serde_json::json!([])),
                        "residuals":verdict.get("residuals").cloned().unwrap_or_else(|| serde_json::json!([])),
                        "instruction":"Reopen your existing Target Intel plan on this same durable chain. Address every material finding, update the plan, execute the missing work, land the resulting observations/receipts, and request a new review only after each close condition is evidenced. Do not fall back to a fixed provider or six-axis worklist."
                    }),
            )
            .await?,
        )
    } else {
        None
    };
    let review_row_version = expected_row_version
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_REVIEW_ROW_VERSION_OVERFLOW"))?;
    tx.commit().await?;
    Ok(RecordedTargetIntelReviewVerdict {
        replayed: false,
        review_row_version,
        hold_id,
        effective_decision: effective_decision.to_string(),
        successor_goal_epoch_id,
    })
}

async fn apply_goal_resume(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source: &ReviewVerdictAuthorityRow,
    source_review_id: Uuid,
    source_hold_id: Option<Uuid>,
    fulfillment_id: Option<Uuid>,
    authority_kind: &str,
    finding_set_sha256: &str,
    recommended_actions_sha256: &str,
    server_message: Value,
) -> Result<Uuid> {
    let controller_worker_run_id = source
        .controller_worker_run_id
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_CONTROLLER_WORKER_MISSING"))?;
    let controller_message_chain_id = source
        .controller_message_chain_id
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_CONTROLLER_CHAIN_MISSING"))?;
    if source.review_fuel_remaining <= 0 {
        bail!("TARGET_INTEL_GOAL_REVIEW_FUEL_EXHAUSTED");
    }
    let successor_goal_epoch = source
        .goal_epoch
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_EPOCH_OVERFLOW"))?;
    let authority_id = deterministic_child_id(
        fulfillment_id.unwrap_or(source_review_id),
        b"goal-resume-authority",
    );
    let successor_goal_epoch_id = deterministic_child_id(authority_id, b"successor-goal-epoch");
    let server_message_sha256 = canonical_sha256(&server_message);
    sqlx::query(
        r#"INSERT INTO target_intel_goal_resume_authorities(
               id,authority_kind,source_review_id,source_hold_id,fulfillment_id,
               operation_id,organization_id,stage_execution_id,stage_run_unit_id,
               scope_snapshot_id,team_plan_id,source_goal_epoch_id,
               source_goal_epoch,successor_goal_epoch_id,successor_goal_epoch,
               controller_work_item_id,controller_worker_run_id,
               controller_message_chain_id,source_plan_row_version,
               source_item_row_version,finding_set_sha256,
               recommended_actions_sha256,material_state_sha256,
               material_actions_sha256,fuel_before,fuel_after,
               server_message,server_message_sha256,status
           ) VALUES(
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
               $18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,'building'
           )"#,
    )
    .bind(authority_id)
    .bind(authority_kind)
    .bind(source_review_id)
    .bind(source_hold_id)
    .bind(fulfillment_id)
    .bind(source.operation_id)
    .bind(source.organization_id)
    .bind(source.stage_execution_id)
    .bind(source.stage_run_unit_id)
    .bind(source.scope_snapshot_id)
    .bind(source.team_plan_id)
    .bind(source.goal_epoch_id)
    .bind(source.goal_epoch)
    .bind(successor_goal_epoch_id)
    .bind(successor_goal_epoch)
    .bind(source.controller_work_item_id)
    .bind(controller_worker_run_id)
    .bind(controller_message_chain_id)
    .bind(source.plan_row_version)
    .bind(source.item_row_version)
    .bind(finding_set_sha256)
    .bind(recommended_actions_sha256)
    .bind(&source.material_state_sha256)
    .bind(&source.material_actions_sha256)
    .bind(source.review_fuel_remaining)
    .bind(source.review_fuel_remaining - 1)
    .bind(&server_message)
    .bind(&server_message_sha256)
    .execute(&mut **tx)
    .await?;
    let reopened = sqlx::query(
        r#"UPDATE stage_team_plans
              SET dispatch_epoch=$1,requests_closed_at=NULL,
                  final_submitter_worker_run_id=NULL,
                  row_version=row_version+1,updated_at=NOW()
            WHERE id=$2 AND row_version=$3 AND dispatch_epoch=$4
              AND requests_closed_at IS NOT NULL
              AND (
                  final_submitter_worker_run_id IS NULL
                  OR final_submitter_worker_run_id=$5
              )"#,
    )
    .bind(successor_goal_epoch)
    .bind(source.team_plan_id)
    .bind(source.plan_row_version)
    .bind(source.goal_epoch)
    .execute(&mut **tx)
    .await?;
    if reopened.rows_affected() != 1 {
        bail!("TARGET_INTEL_GOAL_RESUME_PLAN_CAS_FAILED");
    }
    let item = sqlx::query(
        r#"UPDATE stage_work_items
              SET dispatch_epoch=$1,status='waiting_dependency',
                  row_version=row_version+1,updated_at=NOW()
            WHERE id=$2 AND row_version=$3 AND dispatch_epoch=$4
              AND status IN ('running','waiting_dependency')"#,
    )
    .bind(successor_goal_epoch)
    .bind(source.controller_work_item_id)
    .bind(source.item_row_version)
    .bind(source.goal_epoch)
    .bind(controller_worker_run_id)
    .execute(&mut **tx)
    .await?;
    if item.rows_affected() != 1 {
        bail!("TARGET_INTEL_GOAL_RESUME_CONTROLLER_ITEM_CAS_FAILED");
    }
    let mut chain: Value = sqlx::query_scalar(
        r#"SELECT chain FROM message_chains
            WHERE id=$1 AND task_id=$2 FOR UPDATE"#,
    )
    .bind(controller_message_chain_id)
    .bind(source.operation_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_CONTROLLER_CHAIN_MISSING"))?;
    let messages = chain
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_CONTROLLER_CHAIN_INVALID"))?;
    let continuation_text = format!(
        "[trusted_target_intel_review_continuation]\n{}",
        serde_json::to_string(&server_message)?
    );
    messages.push(serde_json::json!({
        "role":"user",
        "content":[{
            "type":"text",
            "text":continuation_text
        }]
    }));
    let chain_updated = sqlx::query(
        r#"UPDATE message_chains SET chain=$3,updated_at=NOW()
            WHERE id=$1 AND task_id=$2"#,
    )
    .bind(controller_message_chain_id)
    .bind(source.operation_id)
    .bind(&chain)
    .execute(&mut **tx)
    .await?;
    if chain_updated.rows_affected() != 1 {
        bail!("TARGET_INTEL_GOAL_CONTROLLER_CHAIN_UPDATE_FAILED");
    }
    let worker = sqlx::query(
        r#"UPDATE stage_worker_runs
              SET status='waiting_background',checkpoint=$2,
                  checkpoint_version=checkpoint_version+1,
                  lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                  lease_expires_at=NULL,heartbeat_at=NULL,
                  active_tool_call_id=NULL,active_tool_started_at=NULL,
                  terminal_at=NULL,updated_at=NOW()
            WHERE id=$1 AND work_item_id=$3 AND operation_id=$4
              AND stage_execution_id=$5 AND stage_run_unit_id=$6
              AND organization_id=$7 AND message_chain_id=$8
              AND status IN ('running','waiting_background')
              AND active_tool_call_id IS NULL"#,
    )
    .bind(controller_worker_run_id)
    .bind(&chain)
    .bind(source.controller_work_item_id)
    .bind(source.operation_id)
    .bind(source.stage_execution_id)
    .bind(source.stage_run_unit_id)
    .bind(source.organization_id)
    .bind(controller_message_chain_id)
    .execute(&mut **tx)
    .await?;
    if worker.rows_affected() != 1 {
        bail!("TARGET_INTEL_GOAL_RESUME_CONTROLLER_WORKER_CAS_FAILED");
    }
    let source_epoch = sqlx::query(
        r#"UPDATE target_intel_goal_epochs
              SET status='superseded',terminal_at=COALESCE(terminal_at,NOW()),
                  row_version=row_version+1
            WHERE id=$1 AND row_version=$2
              AND status IN ('sealed_for_review','held')"#,
    )
    .bind(source.goal_epoch_id)
    .bind(source.epoch_row_version)
    .execute(&mut **tx)
    .await?;
    if source_epoch.rows_affected() != 1 {
        bail!("TARGET_INTEL_GOAL_RESUME_SOURCE_EPOCH_CAS_FAILED");
    }
    sqlx::query(
        r#"INSERT INTO target_intel_goal_epochs(
               id,operation_id,organization_id,team_plan_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,epoch,status,
               review_fuel_remaining,resume_authority_id,
               controller_work_item_id,controller_worker_run_id,
               controller_message_chain_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'open',$9,$10,$11,$12,$13)"#,
    )
    .bind(successor_goal_epoch_id)
    .bind(source.operation_id)
    .bind(source.organization_id)
    .bind(source.team_plan_id)
    .bind(source.stage_execution_id)
    .bind(source.stage_run_unit_id)
    .bind(source.scope_snapshot_id)
    .bind(successor_goal_epoch)
    .bind(source.review_fuel_remaining - 1)
    .bind(authority_id)
    .bind(source.controller_work_item_id)
    .bind(controller_worker_run_id)
    .bind(controller_message_chain_id)
    .execute(&mut **tx)
    .await?;
    let applied = sqlx::query(
        r#"UPDATE target_intel_goal_resume_authorities
              SET status='applied',row_version=row_version+1,applied_at=NOW()
            WHERE id=$1 AND status='building' AND row_version=0"#,
    )
    .bind(authority_id)
    .execute(&mut **tx)
    .await?;
    if applied.rows_affected() != 1 {
        bail!("TARGET_INTEL_GOAL_RESUME_AUTHORITY_APPLY_FAILED");
    }
    Ok(successor_goal_epoch_id)
}

#[derive(Debug, Clone, PartialEq)]
pub struct FulfillTargetIntelGoalHold {
    pub fulfillment_id: Uuid,
    pub hold_id: Uuid,
    pub expected_hold_row_version: i64,
    pub fulfillment_kind: String,
    pub authority_ref: String,
    pub material_input: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FulfilledTargetIntelGoalHold {
    pub fulfillment_id: Uuid,
    pub successor_goal_epoch_id: Uuid,
    pub successor_goal_epoch: i64,
    pub controller_work_item_id: Uuid,
    pub controller_worker_run_id: Uuid,
    pub controller_message_chain_id: Uuid,
    pub replayed: bool,
}

pub async fn fulfill_hold_and_resume(
    pool: &PgPool,
    input: &FulfillTargetIntelGoalHold,
) -> Result<FulfilledTargetIntelGoalHold> {
    if input.fulfillment_id.is_nil()
        || input.hold_id.is_nil()
        || input.expected_hold_row_version < 0
        || input.fulfillment_kind.trim().is_empty()
        || input.authority_ref.trim().is_empty()
        || !input.material_input.is_object()
    {
        bail!("TARGET_INTEL_GOAL_HOLD_FULFILLMENT_INVALID");
    }
    let mut tx = pool.begin().await?;
    let (requirement_kind, hold_status, hold_row_version, review_id) =
        sqlx::query_as::<_, (String, String, i64, Uuid)>(
            r#"SELECT requirement_kind,status,row_version,review_id
                 FROM target_intel_goal_holds
                WHERE id=$1 FOR UPDATE"#,
        )
        .bind(input.hold_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_HOLD_MISSING"))?;
    let source = sqlx::query_as::<_, ReviewVerdictAuthorityRow>(
        r#"SELECT review.reviewer_worker_run_id,review.status,review.row_version,
                  review.bundle_sha256,review.verdict,review.verdict_sha256,
                  review.operation_id,review.organization_id,
                  review.stage_execution_id,review.stage_run_unit_id,
                  review.scope_snapshot_id,review.team_plan_id,
                  review.goal_epoch_id,review.goal_epoch,
                  review.review_generation,review.round,
                  review.controller_work_item_id,review.controller_worker_run_id,
                  review.controller_message_chain_id,
                  review.material_state_sha256,review.material_actions_sha256,
                  contract.runtime_mode,contract.max_review_rounds,
                  epoch.review_fuel_remaining,epoch.status AS epoch_status,
                  epoch.row_version AS epoch_row_version,
                  plan.row_version AS plan_row_version,
                  item.row_version AS item_row_version,
                  plan.dispatch_epoch AS plan_dispatch_epoch,
                  plan.requests_closed_at IS NOT NULL AS plan_is_closed,
                  plan.final_submitter_worker_run_id IS NULL AS final_submitter_is_unbound
             FROM target_intel_goal_holds hold
             JOIN target_intel_goal_reviews review ON review.id=hold.review_id
             JOIN target_intel_goal_operation_contracts contract
               ON contract.operation_id=review.operation_id
             JOIN target_intel_goal_epochs epoch ON epoch.id=review.goal_epoch_id
             JOIN stage_team_plans plan ON plan.id=review.team_plan_id
             JOIN stage_work_items item ON item.id=review.controller_work_item_id
            WHERE review.id=$1
            FOR UPDATE OF review,epoch,plan,item"#,
    )
    .bind(review_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_REVIEW_MISSING"))?;
    let allowed_kind = match requirement_kind.as_str() {
        "credential" => "credential_provided",
        "scope_confirmation" => "scope_confirmed",
        "subject_confirmation" => "subject_confirmed",
        "provider_recovery" => "provider_recovered",
        "review_fixed_point" | "review_fuel_exhausted" => "operator_override",
        _ => bail!("TARGET_INTEL_GOAL_HOLD_REQUIREMENT_INVALID"),
    };
    if input.fulfillment_kind != allowed_kind {
        bail!("TARGET_INTEL_GOAL_HOLD_FULFILLMENT_KIND_MISMATCH");
    }
    if hold_status == "fulfilled" {
        let replay = sqlx::query_as::<_, (Uuid, String, String, Value, Uuid, i64)>(
            r#"SELECT fulfillment.id,fulfillment.fulfillment_kind,
                      fulfillment.authority_ref,fulfillment.material_input,
                      authority.successor_goal_epoch_id,authority.successor_goal_epoch
                 FROM target_intel_goal_hold_fulfillments fulfillment
                 JOIN target_intel_goal_resume_authorities authority
                   ON authority.fulfillment_id=fulfillment.id AND authority.status='applied'
                WHERE fulfillment.hold_id=$1"#,
        )
        .bind(input.hold_id)
        .fetch_one(&mut *tx)
        .await?;
        if replay.0 != input.fulfillment_id
            || replay.1 != input.fulfillment_kind
            || replay.2 != input.authority_ref
            || replay.3 != input.material_input
        {
            bail!("TARGET_INTEL_GOAL_HOLD_FULFILLMENT_REPLAY_MISMATCH");
        }
        tx.commit().await?;
        return Ok(FulfilledTargetIntelGoalHold {
            fulfillment_id: replay.0,
            successor_goal_epoch_id: replay.4,
            successor_goal_epoch: replay.5,
            controller_work_item_id: source.controller_work_item_id,
            controller_worker_run_id: source
                .controller_worker_run_id
                .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_CONTROLLER_WORKER_MISSING"))?,
            controller_message_chain_id: source
                .controller_message_chain_id
                .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_CONTROLLER_CHAIN_MISSING"))?,
            replayed: true,
        });
    }
    if hold_status != "open"
        || hold_row_version != input.expected_hold_row_version
        || source.status != "needs_human"
        || source.epoch_status != "held"
        || !source.plan_is_closed
        || !source.final_submitter_authority_valid
        || source.plan_dispatch_epoch != source.goal_epoch
    {
        bail!("TARGET_INTEL_GOAL_HOLD_FULFILLMENT_CAS_FAILED");
    }
    sqlx::query(
        r#"INSERT INTO target_intel_goal_hold_fulfillments(
               id,hold_id,expected_hold_row_version,fulfillment_kind,
               authority_ref,material_input
           ) VALUES($1,$2,$3,$4,$5,$6)"#,
    )
    .bind(input.fulfillment_id)
    .bind(input.hold_id)
    .bind(input.expected_hold_row_version)
    .bind(&input.fulfillment_kind)
    .bind(input.authority_ref.trim())
    .bind(&input.material_input)
    .execute(&mut *tx)
    .await?;
    let hold = sqlx::query(
        r#"UPDATE target_intel_goal_holds
              SET status='fulfilled',fulfilled_at=NOW(),row_version=row_version+1
            WHERE id=$1 AND status='open' AND row_version=$2"#,
    )
    .bind(input.hold_id)
    .bind(input.expected_hold_row_version)
    .execute(&mut *tx)
    .await?;
    if hold.rows_affected() != 1 {
        bail!("TARGET_INTEL_GOAL_HOLD_FULFILLMENT_CAS_FAILED");
    }
    let finding_set_sha256: String =
        sqlx::query_scalar("SELECT finding_set_sha256 FROM target_intel_goal_reviews WHERE id=$1")
            .bind(review_id)
            .fetch_one(&mut *tx)
            .await?;
    let recommended_actions_sha256: String = sqlx::query_scalar(
        "SELECT recommended_actions_sha256 FROM target_intel_goal_reviews WHERE id=$1",
    )
    .bind(review_id)
    .fetch_one(&mut *tx)
    .await?;
    let successor_goal_epoch_id = apply_goal_resume(
        &mut tx,
        &source,
        review_id,
        Some(input.hold_id),
        Some(input.fulfillment_id),
        "human_fulfillment",
        &finding_set_sha256,
        &recommended_actions_sha256,
        serde_json::json!({
            "kind":"target_intel_human_fulfillment",
            "hold_id":input.hold_id,
            "fulfillment_id":input.fulfillment_id,
            "fulfillment_kind":input.fulfillment_kind,
            "authority_ref":input.authority_ref,
            "material_input":input.material_input,
        }),
    )
    .await?;
    let successor_goal_epoch = source.goal_epoch + 1;
    let controller_worker_run_id = source
        .controller_worker_run_id
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_CONTROLLER_WORKER_MISSING"))?;
    let controller_message_chain_id = source
        .controller_message_chain_id
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_CONTROLLER_CHAIN_MISSING"))?;
    tx.commit().await?;
    Ok(FulfilledTargetIntelGoalHold {
        fulfillment_id: input.fulfillment_id,
        successor_goal_epoch_id,
        successor_goal_epoch,
        controller_work_item_id: source.controller_work_item_id,
        controller_worker_run_id,
        controller_message_chain_id,
        replayed: false,
    })
}
