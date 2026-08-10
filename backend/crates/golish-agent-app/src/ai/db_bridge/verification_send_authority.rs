//! Production persistence adapter for the Plan C host-owned per-hop send gate.
//!
//! This is intentionally separate from the campaign orchestration repository:
//! it exposes only the two short compounds needed immediately before network
//! I/O and never returns the private prepared-action manifest outside the host.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::harness::{
    admit_investigation_operator_tool, load_embedded_investigation_tool_catalog,
    AdmittedInvestigationOperatorToolV1, InvestigationOperatorToolCatalogV1,
    InvestigationToolAdmissionRejectionV1, InvestigationToolAdmissionRequestV1,
    InvestigationToolContractStatusV1,
};
use golish_agent_runtime::agentic_loop::verification_campaign::{
    BudgetAxisV1, BudgetHeadSnapshotV1, CampaignDispatchAuthoritySnapshotV1,
    CredentialAuthoritySnapshotV1, FrozenSendAuthorizationV1, HierarchicalBudgetSnapshotV1,
    HostCredentialInjector, HostPerHopAuthorityContextV1, HostPerHopAuthorityRepository,
    PreparedActionSendSelectorV1, SendAuthorityError, SystemPinnedResolver, TrustedGetLimitsV1,
    TrustedHttpObservationV1,
};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct PersistedManifest {
    operation_id: Uuid,
    organization_id: Uuid,
    campaign_id: Uuid,
    exact_target_url: String,
    credential: Option<PersistedCredential>,
    network_policy: PersistedNetworkPolicy,
}

#[derive(Debug, Deserialize)]
struct PersistedCredential {
    handle_id: Uuid,
    handle_version: u32,
    revocation_generation: i64,
    injection_origin: String,
    injection_contract_version: String,
}

#[derive(Debug, Deserialize)]
struct PersistedNetworkPolicy {
    exact_origin: String,
    path_boundary: String,
    allowed_destination_set: Vec<String>,
    max_redirect_hops: u8,
    scope_exception_hash: Option<String>,
}

/// Private facts reconstructed from one exact durable Prepared Action chain.
///
/// This type is deliberately not part of a command/IPC boundary: callers only
/// provide UUID selectors, while the production adapter below computes every
/// boolean from the Prepared Action, JIT receipt, current target/scope, budget
/// reservation, conflict-key lease and capability assessment rows.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DurableCatalogAdmissionFactsV1 {
    action_kind: String,
    manifest_capability_id: String,
    manifest_tool_config_id: Option<String>,
    manifest_target_kind: Option<String>,
    manifest_adapter_contract_id: Option<String>,
    manifest_adapter_contract_digest: Option<String>,
    exact_scope_authority: bool,
    target_write_guard_passed: bool,
    fuel_reserved: bool,
    lease_current: bool,
    jit_authorized: bool,
    adapter_authority_current: bool,
    exact_credential_grant: bool,
    external_service_authorized: bool,
}

fn catalog_rejection(reason_code: &'static str) -> InvestigationToolAdmissionRejectionV1 {
    InvestigationToolAdmissionRejectionV1 { reason_code }
}

/// Returns `Ok(None)` only for the pre-existing closed trusted-GET capability
/// family. A capability present in the Investigation operator catalog can
/// never fall through to that transport, even if its persisted binding drifts.
fn derive_catalog_prepared_action_admission(
    catalog: &InvestigationOperatorToolCatalogV1,
    facts: &DurableCatalogAdmissionFactsV1,
) -> Result<Option<AdmittedInvestigationOperatorToolV1>, InvestigationToolAdmissionRejectionV1> {
    let action_profile = catalog
        .tools
        .iter()
        .find(|profile| profile.capability == facts.action_kind);
    let manifest_profile = catalog
        .tools
        .iter()
        .find(|profile| profile.capability == facts.manifest_capability_id);
    let profile = match (action_profile, manifest_profile) {
        (None, None) => return Ok(None),
        (Some(action), Some(manifest))
            if action.tool_config_id == manifest.tool_config_id
                && action.capability == manifest.capability =>
        {
            action
        }
        _ => return Err(catalog_rejection("catalog_action_identity_drift")),
    };

    // Preserve the catalog's authoritative contract-pending/disabled reason
    // before considering any future ready-only persisted binding fields.
    if profile.contract_status != InvestigationToolContractStatusV1::Ready {
        return admit_investigation_operator_tool(
            catalog,
            &InvestigationToolAdmissionRequestV1 {
                tool_config_id: profile.tool_config_id.clone(),
                actor_is_cognitive: false,
                target_kind: facts.manifest_target_kind.clone().unwrap_or_default(),
                exact_scope_authority: facts.exact_scope_authority,
                target_write_guard_passed: facts.target_write_guard_passed,
                fuel_reserved: facts.fuel_reserved,
                lease_current: facts.lease_current,
                jit_authorized: facts.jit_authorized,
                exact_credential_grant: facts.exact_credential_grant,
                external_service_authorized: facts.external_service_authorized,
                adapter_contract_id: facts
                    .manifest_adapter_contract_id
                    .clone()
                    .unwrap_or_default(),
                adapter_contract_digest: facts
                    .manifest_adapter_contract_digest
                    .clone()
                    .unwrap_or_default(),
            },
        )
        .map(Some);
    }

    if facts.manifest_tool_config_id.as_deref() != Some(profile.tool_config_id.as_str())
        || facts
            .manifest_target_kind
            .as_ref()
            .is_none_or(|kind| !profile.target_kinds.contains(kind))
    {
        return Err(catalog_rejection("catalog_binding_missing_or_drifted"));
    }
    if !facts.adapter_authority_current {
        return Err(catalog_rejection("adapter_authority_drift"));
    }

    admit_investigation_operator_tool(
        catalog,
        &InvestigationToolAdmissionRequestV1 {
            tool_config_id: profile.tool_config_id.clone(),
            actor_is_cognitive: false,
            target_kind: facts
                .manifest_target_kind
                .clone()
                .expect("ready catalog binding checked above"),
            exact_scope_authority: facts.exact_scope_authority,
            target_write_guard_passed: facts.target_write_guard_passed,
            fuel_reserved: facts.fuel_reserved,
            lease_current: facts.lease_current,
            jit_authorized: facts.jit_authorized,
            exact_credential_grant: facts.exact_credential_grant,
            external_service_authorized: facts.external_service_authorized,
            adapter_contract_id: facts
                .manifest_adapter_contract_id
                .clone()
                .unwrap_or_default(),
            adapter_contract_digest: facts
                .manifest_adapter_contract_digest
                .clone()
                .unwrap_or_default(),
        },
    )
    .map(Some)
}

#[derive(Clone)]
pub struct PgPreparedActionSendAuthorityRepository {
    pool: Arc<PgPool>,
}

impl PgPreparedActionSendAuthorityRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

/// Exact-version vault decorator for trusted verification transports.  The
/// query re-derives the immutable action/authorization/credential relation;
/// the caller cannot select a vault entry or supply a header name/value.
#[derive(Clone)]
pub struct PgVaultCredentialInjector {
    pool: Arc<PgPool>,
}

impl PgVaultCredentialInjector {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

/// Production trusted GET transport assembled only from host-owned adapters.
/// Its public input is limited to durable selectors, a previously compiled
/// URL, and bounded limits; authority, DNS and secrets are always re-derived.
#[derive(Clone)]
pub struct TrustedVerificationHttpTransport {
    authority: PgPreparedActionSendAuthorityRepository,
    credentials: PgVaultCredentialInjector,
}

impl TrustedVerificationHttpTransport {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            authority: PgPreparedActionSendAuthorityRepository::new(pool.clone()),
            credentials: PgVaultCredentialInjector::new(pool),
        }
    }

    pub async fn execute_get(
        &self,
        selector: PreparedActionSendSelectorV1,
    ) -> Result<TrustedHttpObservationV1, SendAuthorityError> {
        let (compiled_url, max_response_bytes_per_hop, max_wall_clock_ms_per_hop): (
            String,
            i64,
            i64,
        ) = sqlx::query_as(
            r#"SELECT action.private_manifest #>> '{exact_target_url}',
                          response_axis.axis_limit,wall_axis.axis_limit
                     FROM verification_action_executions execution
                     JOIN verification_prepared_actions action
                       ON action.prepared_action_id=execution.prepared_action_id
                      AND action.operation_id=execution.operation_id
                     JOIN verification_prepared_action_authorizations authorization
                       ON authorization.authorization_receipt_id=execution.authorization_receipt_id
                      AND authorization.prepared_action_id=action.prepared_action_id
                      AND authorization.campaign_id=action.campaign_id
                      AND authorization.decision='authorized'
                     JOIN verification_budget_contracts budget
                       ON budget.scope_kind='action' AND budget.scope_id=action.prepared_action_id
                      AND budget.contract_hash=action.upper_budget_set_hash
                      AND budget.sealed_at IS NOT NULL
                     JOIN verification_budget_contract_axes response_axis
                       ON response_axis.budget_contract_id=budget.budget_contract_id
                      AND response_axis.axis_kind='response_bytes'
                     JOIN verification_budget_contract_axes wall_axis
                       ON wall_axis.budget_contract_id=budget.budget_contract_id
                      AND wall_axis.axis_kind='wall_clock_ms'
                    WHERE execution.action_execution_id=$1
                      AND execution.operation_id=$2 AND action.campaign_id=$3
                      AND execution.prepared_action_id=$4
                      AND execution.authorization_receipt_id=$5
                      AND execution.state='started'
                      AND execution.completed_at IS NULL"#,
        )
        .bind(selector.action_execution_id)
        .bind(selector.operation_id)
        .bind(selector.campaign_id)
        .bind(selector.prepared_action_id)
        .bind(selector.authorization_receipt_id)
        .fetch_optional(&*self.authority.pool)
        .await
        .map_err(|_| SendAuthorityError::AuthorityQuarantined)?
        .ok_or(SendAuthorityError::AuthorityQuarantined)?;
        let limits = TrustedGetLimitsV1 {
            max_response_bytes_per_hop: u64::try_from(max_response_bytes_per_hop)
                .map_err(|_| SendAuthorityError::BudgetExhausted)?,
            max_wall_clock_ms_per_hop: u64::try_from(max_wall_clock_ms_per_hop)
                .map_err(|_| SendAuthorityError::BudgetExhausted)?,
        };
        golish_agent_runtime::agentic_loop::verification_campaign::execute_trusted_get_v1(
            &self.authority,
            &SystemPinnedResolver,
            Some(&self.credentials),
            selector,
            compiled_url,
            limits,
        )
        .await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecuteAuthorizedPreparedActionV1 {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub prepared_action_id: Uuid,
    pub authorization_receipt_id: Uuid,
    pub action_execution_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedPreparedActionV1 {
    pub capability_execution_receipt_id: Uuid,
    pub oracle_assessment_id: Uuid,
    pub terminal_state: String,
    pub transport_reason_code: Option<String>,
    pub replayed: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct DurableCatalogAdmissionRow {
    action_kind: String,
    manifest_capability_id: String,
    manifest_tool_config_id: Option<String>,
    manifest_target_kind: Option<String>,
    manifest_adapter_contract_id: Option<String>,
    manifest_adapter_contract_digest: Option<String>,
    exact_scope_authority: bool,
    target_write_guard_passed: bool,
    fuel_reserved: bool,
    lease_current: bool,
    jit_authorized: bool,
    adapter_authority_current: bool,
}

/// Production catalog routing uses UUID selectors only. The query reconstructs
/// the catalog binding and every launch authority from durable rows; no model,
/// IPC caller or scheduler boolean is accepted as an admission fact.
async fn rederive_catalog_prepared_action_admission(
    pool: &PgPool,
    selector: PreparedActionSendSelectorV1,
) -> Result<
    Result<Option<AdmittedInvestigationOperatorToolV1>, InvestigationToolAdmissionRejectionV1>,
    SendAuthorityError,
> {
    let row = sqlx::query_as::<_, DurableCatalogAdmissionRow>(
        r#"SELECT action.action_kind,
                  COALESCE(action.private_manifest->>'capability_id','')
                      AS manifest_capability_id,
                  action.private_manifest #>> '{operator_tool,tool_config_id}'
                      AS manifest_tool_config_id,
                  action.private_manifest #>> '{operator_tool,target_kind}'
                      AS manifest_target_kind,
                  action.private_manifest->>'adapter_contract_version'
                      AS manifest_adapter_contract_id,
                  action.private_manifest->>'adapter_contract_digest'
                      AS manifest_adapter_contract_digest,
                  (
                      action.private_manifest->>'operation_id'=action.operation_id::TEXT
                      AND action.private_manifest->>'organization_id'=action.organization_id::TEXT
                      AND action.private_manifest->>'campaign_id'=action.campaign_id::TEXT
                      AND action.private_manifest->>'target_id'=action.target_live_id::TEXT
                      AND authorization.expected_private_manifest_hash=action.private_manifest_hash
                      AND authorization.reviewed_action_hash=action.display_projection_hash
                      AND execution.operation_id=action.operation_id
                      AND execution.project_scope_id=action.project_scope_id
                      AND execution.organization_id=action.organization_id
                  ) AS exact_scope_authority,
                  (
                      target.id IS NOT NULL
                      AND target.id=action.target_live_id
                      AND target.organization_id=action.organization_id
                      AND target.target_type::TEXT=action.target_type_at_time
                      AND target.value=action.target_value_at_time
                      AND target.scope::TEXT='in'
                  ) AS target_write_guard_passed,
                  (reservation.state='active') AS fuel_reserved,
                  (
                      conflict_set.sealed_at IS NOT NULL
                      AND conflict_set.member_count=(
                          SELECT COUNT(*)
                            FROM verification_action_conflict_set_members member
                           WHERE member.conflict_set_id=conflict_set.conflict_set_id
                      )
                      AND conflict_set.member_count=(
                          SELECT COUNT(*)
                            FROM verification_action_conflict_set_members member
                            JOIN verification_conflict_key_heads head
                              ON head.operation_id=action.operation_id
                             AND head.project_scope_id=action.project_scope_id
                             AND head.organization_id=action.organization_id
                             AND head.key_kind=member.key_kind
                             AND head.key_identity_hash=member.key_identity_hash
                             AND head.state='active'
                             AND head.owner_campaign_id=action.campaign_id
                             AND head.owner_prepared_action_id=action.prepared_action_id
                           WHERE member.conflict_set_id=conflict_set.conflict_set_id
                      )
                  ) AS lease_current,
                  (
                      action.state='started'
                      AND execution.state='started' AND execution.completed_at IS NULL
                      AND authorization.decision='authorized'
                      AND authorization.expires_at>statement_timestamp()
                      AND execution.campaign_dispatch_generation=
                          authorization.campaign_dispatch_generation
                  ) AS jit_authorized,
                  (
                      assessment.status='available'
                      AND assessment.capability_key=action.action_kind
                      AND assessment.adapter_contract_version=
                          action.private_manifest->>'adapter_contract_version'
                      AND assessment.adapter_contract_digest=
                          action.private_manifest->>'adapter_contract_digest'
                      AND NOT EXISTS(
                          SELECT 1
                            FROM verification_capability_assessments newer
                           WHERE newer.hypothesis_revision_id=assessment.hypothesis_revision_id
                             AND newer.verification_objective_id=
                                 assessment.verification_objective_id
                             AND newer.capability_key=assessment.capability_key
                             AND newer.assessment_ordinal>assessment.assessment_ordinal
                      )
                  ) AS adapter_authority_current
             FROM verification_prepared_actions action
             JOIN verification_prepared_action_authorizations authorization
               ON authorization.authorization_receipt_id=$4
              AND authorization.prepared_action_id=action.prepared_action_id
              AND authorization.campaign_id=action.campaign_id
              AND authorization.operation_id=action.operation_id
              AND authorization.project_scope_id=action.project_scope_id
              AND authorization.organization_id=action.organization_id
             JOIN verification_action_executions execution
               ON execution.action_execution_id=$5
              AND execution.prepared_action_id=action.prepared_action_id
              AND execution.authorization_receipt_id=authorization.authorization_receipt_id
             JOIN verification_budget_reservations reservation
               ON reservation.budget_reservation_id=execution.budget_reservation_id
              AND reservation.prepared_action_id=action.prepared_action_id
              AND reservation.authorization_receipt_id=authorization.authorization_receipt_id
             JOIN verification_action_conflict_sets conflict_set
               ON conflict_set.conflict_set_id=execution.conflict_set_id
              AND conflict_set.prepared_action_id=action.prepared_action_id
             JOIN verification_capability_assessments assessment
               ON assessment.assessment_id=action.capability_assessment_id
              AND assessment.operation_id=action.operation_id
              AND assessment.project_scope_id=action.project_scope_id
              AND assessment.organization_id=action.organization_id
             LEFT JOIN project_scopes scope
               ON scope.project_scope_id=action.project_scope_id
              AND scope.retired_at IS NULL
             LEFT JOIN targets target
               ON target.id=action.target_live_id
              AND target.project_path=scope.canonical_project_path
            WHERE action.operation_id=$1 AND action.campaign_id=$2
              AND action.prepared_action_id=$3"#,
    )
    .bind(selector.operation_id)
    .bind(selector.campaign_id)
    .bind(selector.prepared_action_id)
    .bind(selector.authorization_receipt_id)
    .bind(selector.action_execution_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| SendAuthorityError::AuthorityQuarantined)?
    .ok_or(SendAuthorityError::AuthorityQuarantined)?;

    let catalog = load_embedded_investigation_tool_catalog()
        .map_err(|_| SendAuthorityError::AuthorityQuarantined)?;
    let catalog_backed = catalog.tools.iter().any(|profile| {
        profile.capability == row.action_kind || profile.capability == row.manifest_capability_id
    });
    if !catalog_backed {
        return Ok(Ok(None));
    }

    // This strong read rechecks current dispatch/quarantine, JIT expiry,
    // credential generation and all four budget heads before catalog routing.
    let current = PgPreparedActionSendAuthorityRepository::new(Arc::new(pool.clone()))
        .read_current_send_authority(selector)
        .await?;
    let exact_credential_grant = current
        .credential
        .as_ref()
        .is_none_or(|credential| !credential.revoked);
    Ok(derive_catalog_prepared_action_admission(
        &catalog,
        &DurableCatalogAdmissionFactsV1 {
            action_kind: row.action_kind,
            manifest_capability_id: row.manifest_capability_id,
            manifest_tool_config_id: row.manifest_tool_config_id,
            manifest_target_kind: row.manifest_target_kind,
            manifest_adapter_contract_id: row.manifest_adapter_contract_id,
            manifest_adapter_contract_digest: row.manifest_adapter_contract_digest,
            exact_scope_authority: row.exact_scope_authority,
            target_write_guard_passed: row.target_write_guard_passed,
            fuel_reserved: row.fuel_reserved,
            lease_current: row.lease_current,
            jit_authorized: row.jit_authorized,
            adapter_authority_current: row.adapter_authority_current,
            exact_credential_grant,
            // No durable external-service grant table is part of this rollout.
            // External/OAST members therefore remain fail-closed.
            external_service_authorized: false,
        },
    ))
}

/// Executes the post-JIT host compound for one already-durably-started action.
///
/// The stable request is an idempotency fence, never a permission token. On a
/// replay with a running Tool Truth receipt we deliberately do not send again:
/// the receipt is closed as outcome-unknown and the oracle remains
/// inconclusive. V1 stores bounded transport metadata only, so even a
/// successful HTTP exchange cannot be upgraded to proof/refutation until a
/// capability-specific complete raw-witness finalizer is installed.
pub async fn execute_authorized_prepared_action_v1(
    pool: Arc<PgPool>,
    command: ExecuteAuthorizedPreparedActionV1,
) -> anyhow::Result<ExecutedPreparedActionV1> {
    anyhow::ensure!(
        [
            command.stable_request_id,
            command.operation_id,
            command.campaign_id,
            command.prepared_action_id,
            command.authorization_receipt_id,
            command.action_execution_id,
        ]
        .into_iter()
        .all(|id| !id.is_nil()),
        "verification action executor received a nil durable selector"
    );
    let receipt_begin =
        golish_db::repo::verification_prepared_actions::begin_verification_action_capability_receipt(
            pool.as_ref(),
            golish_db::repo::verification_prepared_actions::BeginVerificationActionCapabilityReceipt {
                stable_request_id: Uuid::new_v5(
                    &command.stable_request_id,
                    b"verification-action-capability-receipt-begin.v1",
                ),
                action_execution_id: command.action_execution_id,
                prepared_action_id: command.prepared_action_id,
            },
        )
        .await?;

    let existing_receipt: (
        String,
        Option<chrono::DateTime<chrono::Utc>>,
        serde_json::Value,
    ) = sqlx::query_as(
        r#"SELECT attempt_state,finalized_at,typed_landing
                 FROM capability_execution_receipts WHERE id=$1"#,
    )
    .bind(receipt_begin.capability_execution_receipt_id)
    .fetch_one(pool.as_ref())
    .await?;
    let (terminal_state, observation, transport_reason_code) = if existing_receipt.1.is_some() {
        let observation = existing_receipt
            .2
            .get("observation")
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "finalized verification action receipt is missing its exact observation projection"
                )
            })?;
        (
            existing_receipt.0,
            observation,
            Some("finalized_receipt_replay".to_owned()),
        )
    } else if receipt_begin.replayed {
        (
            "outcome_unknown".to_owned(),
            serde_json::json!({
                "contract_version": "verification-action-observation.v1",
                "witness_completeness": "metadata_only",
                "recovery_disposition": "durable_begin_without_terminal_receipt",
            }),
            Some("durable_begin_response_loss".to_owned()),
        )
    } else {
        let selector = PreparedActionSendSelectorV1 {
            operation_id: command.operation_id,
            campaign_id: command.campaign_id,
            prepared_action_id: command.prepared_action_id,
            authorization_receipt_id: command.authorization_receipt_id,
            action_execution_id: command.action_execution_id,
        };
        let catalog_admission =
            rederive_catalog_prepared_action_admission(pool.as_ref(), selector).await;
        match catalog_admission {
            Ok(Ok(None)) => match TrustedVerificationHttpTransport::new(pool.clone())
                .execute_get(selector)
                .await
            {
                Ok(observation) => {
                    let hops = observation
                        .hops
                        .into_iter()
                        .map(|hop| {
                            serde_json::json!({
                                "url": hop.url,
                                "status": hop.status,
                                "response_bytes": hop.response_bytes,
                                "body_sha256": hop.body_sha256,
                                "content_type": hop.content_type,
                            })
                        })
                        .collect::<Vec<_>>();
                    (
                        "succeeded".to_owned(),
                        serde_json::json!({
                            "contract_version": "verification-action-observation.v1",
                            "witness_completeness": "metadata_only",
                            "final_url": observation.final_url,
                            "hops": hops,
                        }),
                        None,
                    )
                }
                Err(error) => (
                    "failed".to_owned(),
                    serde_json::json!({
                        "contract_version": "verification-action-observation.v1",
                        "witness_completeness": "metadata_only",
                        "transport_reason_code": error.code(),
                    }),
                    Some(error.code().to_owned()),
                ),
            },
            Ok(Ok(Some(admitted))) => {
                // Admission does not equal dispatch. Until the exact typed
                // Operator adapter is installed at this host boundary, a
                // catalog member must never inherit the trusted-GET transport.
                let reason = "investigation_typed_operator_dispatch_unavailable";
                (
                    "failed".to_owned(),
                    serde_json::json!({
                        "contract_version": "verification-action-observation.v1",
                        "witness_completeness": "metadata_only",
                        "transport_reason_code": reason,
                        "tool_config_id": admitted.tool_config_id,
                        "capability": admitted.capability,
                    }),
                    Some(reason.to_owned()),
                )
            }
            Ok(Err(rejection)) => {
                let reason = rejection.reason_code;
                (
                    "failed".to_owned(),
                    serde_json::json!({
                        "contract_version": "verification-action-observation.v1",
                        "witness_completeness": "metadata_only",
                        "transport_reason_code": reason,
                    }),
                    Some(reason.to_owned()),
                )
            }
            Err(error) => {
                let reason = error.code();
                (
                    "failed".to_owned(),
                    serde_json::json!({
                        "contract_version": "verification-action-observation.v1",
                        "witness_completeness": "metadata_only",
                        "transport_reason_code": reason,
                    }),
                    Some(reason.to_owned()),
                )
            }
        }
    };
    let landing = golish_db::repo::verification_prepared_actions::finalize_verification_action_semantic_landing(
        pool.as_ref(),
        &golish_db::repo::verification_prepared_actions::FinalizeVerificationActionSemanticLanding {
            stable_request_id: command.stable_request_id,
            operation_id: command.operation_id,
            campaign_id: command.campaign_id,
            prepared_action_id: command.prepared_action_id,
            authorization_receipt_id: command.authorization_receipt_id,
            action_execution_id: command.action_execution_id,
            capability_execution_receipt_id: receipt_begin.capability_execution_receipt_id,
            terminal_state: terminal_state.clone(),
            observation,
        },
    )
    .await?;
    Ok(ExecutedPreparedActionV1 {
        capability_execution_receipt_id: receipt_begin.capability_execution_receipt_id,
        oracle_assessment_id: landing.oracle_assessment_id,
        terminal_state: landing.terminal_state,
        transport_reason_code,
        replayed: receipt_begin.replayed || landing.replayed,
    })
}

#[derive(sqlx::FromRow)]
struct CurrentCredentialSecret {
    entry_type: String,
    value: String,
    username: String,
    injection_contract_version: String,
}

#[async_trait]
impl HostCredentialInjector for PgVaultCredentialInjector {
    async fn inject_exact_origin(
        &self,
        selector: PreparedActionSendSelectorV1,
        exact_origin: &str,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, SendAuthorityError> {
        let row = sqlx::query_as::<_, CurrentCredentialSecret>(
            r#"SELECT vault.entry_type::TEXT AS entry_type,vault.value,vault.username,
                      credential.injection_contract_version
                 FROM verification_action_executions execution
                 JOIN verification_prepared_actions action
                   ON action.prepared_action_id=execution.prepared_action_id
                  AND action.operation_id=execution.operation_id
                  AND action.campaign_id=execution.campaign_id
                 JOIN verification_prepared_action_authorizations authorization
                   ON authorization.authorization_receipt_id=execution.authorization_receipt_id
                  AND authorization.prepared_action_id=action.prepared_action_id
                  AND authorization.decision='authorized'
                 JOIN verification_credential_authority_heads credential
                   ON credential.operation_id=action.operation_id
                  AND credential.handle_id=(action.private_manifest #>> '{credential,handle_id}')::UUID
                 JOIN vault_entries vault ON vault.id=credential.handle_id
                WHERE execution.action_execution_id=$1
                  AND execution.operation_id=$2 AND execution.campaign_id=$3
                  AND execution.prepared_action_id=$4
                  AND execution.authorization_receipt_id=$5
                  AND execution.state='started'
                  AND execution.completed_at IS NULL
                  AND credential.handle_version=(action.private_manifest #>> '{credential,handle_version}')::BIGINT
                  AND credential.revocation_generation=(action.private_manifest #>> '{credential,revocation_generation}')::BIGINT
                  AND credential.injection_origin=$6
                  AND credential.injection_origin=action.private_manifest #>> '{credential,injection_origin}'
                  AND credential.injection_contract_version=action.private_manifest #>> '{credential,injection_contract_version}'
                  AND credential.revoked=FALSE AND vault.status<>'invalid'
                  AND vault.updated_at<=credential.updated_at"#,
        )
        .bind(selector.action_execution_id)
        .bind(selector.operation_id)
        .bind(selector.campaign_id)
        .bind(selector.prepared_action_id)
        .bind(selector.authorization_receipt_id)
        .bind(exact_origin)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|_| SendAuthorityError::CredentialDrift)?
        .ok_or(SendAuthorityError::CredentialDrift)?;
        if row.injection_contract_version != "header.v1" {
            return Err(SendAuthorityError::CredentialDrift);
        }
        let secret = golish_core::vault::deobfuscate(&row.value)
            .map_err(|_| SendAuthorityError::CredentialDrift)?;
        if secret.is_empty() {
            return Err(SendAuthorityError::CredentialDrift);
        }
        match row.entry_type.as_str() {
            "token" => {
                let mut value = reqwest::header::HeaderValue::try_from(format!("Bearer {secret}"))
                    .map_err(|_| SendAuthorityError::CredentialDrift)?;
                value.set_sensitive(true);
                Ok(request.header(reqwest::header::AUTHORIZATION, value))
            }
            "api_key" => {
                let mut value = reqwest::header::HeaderValue::try_from(secret)
                    .map_err(|_| SendAuthorityError::CredentialDrift)?;
                value.set_sensitive(true);
                Ok(request.header("X-API-Key", value))
            }
            "password" if !row.username.is_empty() => {
                Ok(request.basic_auth(row.username, Some(secret)))
            }
            _ => Err(SendAuthorityError::CredentialDrift),
        }
    }
}

fn map_db_error(error: golish_db::DbError) -> SendAuthorityError {
    let detail = error.to_string();
    if detail.contains("VERIFICATION_BUDGET_EXHAUSTED") {
        SendAuthorityError::BudgetExhausted
    } else if detail.contains("CAMPAIGN_DISPATCH_HELD") {
        SendAuthorityError::CampaignDispatchHeld
    } else if detail.contains("GENERATION") {
        SendAuthorityError::CampaignDispatchGenerationDrift
    } else {
        SendAuthorityError::AuthorityQuarantined
    }
}

fn axis_from_db(value: &str) -> Option<BudgetAxisV1> {
    Some(match value {
        "requests" => BudgetAxisV1::Requests,
        "response_bytes" => BudgetAxisV1::ResponseBytes,
        "wall_clock_ms" => BudgetAxisV1::WallClockMs,
        "retries" => BudgetAxisV1::Retries,
        "browser_steps" => BudgetAxisV1::BrowserSteps,
        "oast_tokens" => BudgetAxisV1::OastTokens,
        _ => return None,
    })
}

fn axis_to_db(value: BudgetAxisV1) -> &'static str {
    match value {
        BudgetAxisV1::Requests => "requests",
        BudgetAxisV1::ResponseBytes => "response_bytes",
        BudgetAxisV1::WallClockMs => "wall_clock_ms",
        BudgetAxisV1::Retries => "retries",
        BudgetAxisV1::BrowserSteps => "browser_steps",
        BudgetAxisV1::OastTokens => "oast_tokens",
    }
}

fn non_negative(value: i64) -> Result<u64, SendAuthorityError> {
    u64::try_from(value).map_err(|_| SendAuthorityError::AuthorityQuarantined)
}

fn build_budget_head(
    scope_kind: &str,
    rows: &[golish_db::repo::verification_prepared_actions::PreparedActionBudgetHeadRow],
) -> Result<BudgetHeadSnapshotV1, SendAuthorityError> {
    let mut limits = BTreeMap::new();
    let mut consumed = BTreeMap::new();
    let mut reserved = BTreeMap::new();
    let mut unknown_held = BTreeMap::new();
    let mut reservation_remaining = BTreeMap::new();
    let mut fences = BTreeMap::new();
    let selected = rows
        .iter()
        .filter(|row| row.scope_kind == scope_kind)
        .collect::<Vec<_>>();
    if selected.len() != BudgetAxisV1::ALL.len() {
        return Err(SendAuthorityError::AuthorityQuarantined);
    }
    for row in selected {
        let axis = axis_from_db(&row.axis_kind).ok_or(SendAuthorityError::AuthorityQuarantined)?;
        if limits.insert(axis, non_negative(row.axis_limit)?).is_some()
            || consumed.insert(axis, non_negative(row.consumed)?).is_some()
            || reserved.insert(axis, non_negative(row.reserved)?).is_some()
            || unknown_held
                .insert(axis, non_negative(row.unknown_held)?)
                .is_some()
            || reservation_remaining
                .insert(axis, non_negative(row.reservation_remaining)?)
                .is_some()
            || fences.insert(axis, row.row_version).is_some()
        {
            return Err(SendAuthorityError::AuthorityQuarantined);
        }
    }
    Ok(BudgetHeadSnapshotV1 {
        limits,
        consumed,
        reserved,
        unknown_held,
        reservation_remaining,
        fences,
    })
}

fn exact_credential_snapshot(
    manifest: Option<&PersistedCredential>,
    action: &golish_db::repo::verification_prepared_actions::PreparedActionSendAuthorityRow,
) -> Result<Option<CredentialAuthoritySnapshotV1>, SendAuthorityError> {
    let Some(frozen) = manifest else {
        if action.credential_handle_id.is_some() {
            return Err(SendAuthorityError::CredentialDrift);
        }
        return Ok(None);
    };
    if frozen.handle_id.is_nil()
        || frozen.handle_version == 0
        || frozen.revocation_generation < 0
        || action.credential_handle_id != Some(frozen.handle_id)
        || action.credential_injection_contract_version.as_deref()
            != Some(frozen.injection_contract_version.as_str())
    {
        return Err(SendAuthorityError::CredentialDrift);
    }
    let handle_version = action
        .credential_handle_version
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(SendAuthorityError::CredentialDrift)?;
    let revocation_generation = action
        .credential_revocation_generation
        .ok_or(SendAuthorityError::CredentialDrift)?;
    let injection_origin = action
        .credential_injection_origin
        .clone()
        .ok_or(SendAuthorityError::CredentialDrift)?;
    let vault_changed = match (
        action.credential_authority_updated_at,
        action.credential_vault_updated_at,
    ) {
        (Some(authority_at), Some(vault_at)) => vault_at > authority_at,
        _ => true,
    };
    let vault_invalid = action
        .credential_vault_status
        .as_deref()
        .is_none_or(|status| status == "invalid");
    Ok(Some(CredentialAuthoritySnapshotV1 {
        handle_version,
        revocation_generation,
        revoked: action.credential_revoked.unwrap_or(true) || vault_changed || vault_invalid,
        injection_origin,
    }))
}

#[async_trait]
impl HostPerHopAuthorityRepository for PgPreparedActionSendAuthorityRepository {
    async fn read_current_send_authority(
        &self,
        selector: PreparedActionSendSelectorV1,
    ) -> Result<HostPerHopAuthorityContextV1, SendAuthorityError> {
        let current = golish_db::repo::verification_prepared_actions::load_current_prepared_action_send_authority(
            &self.pool,
            selector.operation_id,
            selector.campaign_id,
            selector.prepared_action_id,
            selector.authorization_receipt_id,
            selector.action_execution_id,
        )
        .await
        .map_err(map_db_error)?;
        let manifest: PersistedManifest =
            serde_json::from_value(current.action.private_manifest.clone())
                .map_err(|_| SendAuthorityError::AuthorityQuarantined)?;
        if manifest.operation_id != current.action.operation_id
            || manifest.organization_id != current.action.organization_id
            || manifest.campaign_id != current.action.campaign_id
            || manifest.credential.as_ref().is_some_and(|credential| {
                credential.injection_origin != manifest.network_policy.exact_origin
            })
        {
            return Err(SendAuthorityError::AuthorityQuarantined);
        }
        let exact_target = url::Url::parse(&manifest.exact_target_url)
            .map_err(|_| SendAuthorityError::DestinationPolicyDenied)?;
        let exact_target_origin = exact_target.origin().ascii_serialization();
        let destination_count = manifest.network_policy.allowed_destination_set.len();
        let allowed_destination_origins = manifest
            .network_policy
            .allowed_destination_set
            .into_iter()
            .collect::<BTreeSet<_>>();
        if allowed_destination_origins.is_empty()
            || allowed_destination_origins.len() != destination_count
            || manifest.network_policy.exact_origin.trim().is_empty()
            || manifest.network_policy.path_boundary.trim().is_empty()
            || exact_target_origin != manifest.network_policy.exact_origin
            || !allowed_destination_origins.contains(&exact_target_origin)
        {
            return Err(SendAuthorityError::DestinationPolicyDenied);
        }
        let credential = exact_credential_snapshot(manifest.credential.as_ref(), &current.action)?;
        let authorization = FrozenSendAuthorizationV1 {
            campaign_dispatch_generation: current.action.campaign_dispatch_generation,
            credential_handle_version: manifest
                .credential
                .as_ref()
                .map(|credential| credential.handle_version),
            credential_revocation_generation: manifest
                .credential
                .as_ref()
                .map(|credential| credential.revocation_generation),
            exact_origin: manifest.network_policy.exact_origin,
            path_boundary: manifest.network_policy.path_boundary,
            allowed_destination_origins,
            max_redirect_hops: manifest.network_policy.max_redirect_hops,
            allow_non_public_destination: manifest.network_policy.scope_exception_hash.is_some(),
            non_public_scope_exception_hash: manifest.network_policy.scope_exception_hash,
            expires_at: current.action.authorization_expires_at,
        };
        let dispatch = CampaignDispatchAuthoritySnapshotV1 {
            campaign_dispatch_held: current.action.campaign_dispatch_held,
            campaign_dispatch_generation: current.action.current_campaign_dispatch_generation,
            operation_admission_held: current.action.operation_admission_held,
            operation_admission_generation: current.action.operation_admission_generation,
            global_row_version: current.action.safety_hold_row_version,
            quarantine_pending: current.action.quarantine_pending,
        };
        let budgets = HierarchicalBudgetSnapshotV1 {
            operation: build_budget_head("operation", &current.budget_heads)?,
            wave: build_budget_head("wave", &current.budget_heads)?,
            campaign: build_budget_head("campaign", &current.budget_heads)?,
            action: build_budget_head("action", &current.budget_heads)?,
        };
        Ok(HostPerHopAuthorityContextV1 {
            authorization,
            dispatch,
            credential,
            budgets,
            checked_at: current.action.checked_at,
        })
    }

    async fn consume_budget_before_send(
        &self,
        selector: PreparedActionSendSelectorV1,
        expected_campaign_dispatch_generation: i64,
        expected_budget_fences: [BTreeMap<BudgetAxisV1, i64>; 4],
        delta: &BTreeMap<BudgetAxisV1, u64>,
    ) -> Result<(), SendAuthorityError> {
        let fences = expected_budget_fences.map(|head| {
            head.into_iter()
                .map(|(axis, fence)| (axis_to_db(axis).to_string(), fence))
                .collect::<BTreeMap<_, _>>()
        });
        let delta = delta
            .iter()
            .map(|(axis, value)| {
                i64::try_from(*value)
                    .map(|value| (axis_to_db(*axis).to_string(), value))
                    .map_err(|_| SendAuthorityError::BudgetExhausted)
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        golish_db::repo::verification_prepared_actions::consume_prepared_action_budget_before_io(
            &self.pool,
            selector.action_execution_id,
            expected_campaign_dispatch_generation,
            fences,
            &delta,
        )
        .await
        .map_err(map_db_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_agent_kit::harness::InvestigationTypedAdapterRefV1;

    fn fully_current_catalog_facts(capability_id: &str) -> DurableCatalogAdmissionFactsV1 {
        DurableCatalogAdmissionFactsV1 {
            action_kind: capability_id.to_owned(),
            manifest_capability_id: capability_id.to_owned(),
            manifest_tool_config_id: Some("arjun".to_owned()),
            manifest_target_kind: Some("endpoint".to_owned()),
            manifest_adapter_contract_id: Some("typed.arjun.v1".to_owned()),
            manifest_adapter_contract_digest: Some("a".repeat(64)),
            exact_scope_authority: true,
            target_write_guard_passed: true,
            fuel_reserved: true,
            lease_current: true,
            jit_authorized: true,
            adapter_authority_current: true,
            exact_credential_grant: true,
            external_service_authorized: false,
        }
    }

    fn ready_arjun_catalog() -> InvestigationOperatorToolCatalogV1 {
        let mut catalog = load_embedded_investigation_tool_catalog().expect("embedded catalog");
        let arjun = catalog
            .tools
            .iter_mut()
            .find(|profile| profile.tool_config_id == "arjun")
            .expect("arjun profile");
        arjun.contract_status = InvestigationToolContractStatusV1::Ready;
        arjun.typed_adapter = Some(InvestigationTypedAdapterRefV1 {
            contract_id: "typed.arjun.v1".to_owned(),
            contract_digest: "a".repeat(64),
        });
        catalog.validate().expect("ready fixture catalog");
        catalog
    }

    #[test]
    fn budget_axis_mapping_is_total_and_stable() {
        for axis in BudgetAxisV1::ALL {
            assert_eq!(axis_from_db(axis_to_db(axis)), Some(axis));
        }
    }

    #[test]
    fn private_manifest_parser_ignores_unneeded_secret_adjacent_fields() {
        let operation_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let campaign_id = Uuid::new_v4();
        let parsed: PersistedManifest = serde_json::from_value(serde_json::json!({
            "operation_id": operation_id,
            "organization_id": organization_id,
            "campaign_id": campaign_id,
            "exact_target_url": "https://example.test/",
            "credential": null,
            "network_policy": {
                "exact_origin": "https://example.test",
                "path_boundary": "/",
                "allowed_destination_set": ["https://example.test"],
                "max_redirect_hops": 0,
                "scope_exception_hash": null,
                "proxy_mode": "none"
            },
            "cleanup_obligations": []
        }))
        .expect("manifest");
        assert_eq!(parsed.operation_id, operation_id);
        assert!(parsed.credential.is_none());
    }

    #[test]
    fn every_embedded_catalog_backed_action_is_fail_closed_before_transport() {
        let catalog = load_embedded_investigation_tool_catalog().expect("embedded catalog");
        for profile in &catalog.tools {
            let mut facts = fully_current_catalog_facts(&profile.capability);
            facts.manifest_tool_config_id = Some(profile.tool_config_id.clone());
            facts.manifest_target_kind = Some(profile.target_kinds[0].clone());
            facts.manifest_adapter_contract_id = Some("forged.adapter.v1".to_owned());
            let denied = derive_catalog_prepared_action_admission(&catalog, &facts)
                .expect_err("contract-pending/disabled catalog member must fail closed");
            assert_eq!(denied.reason_code, "tool_contract_not_ready");
        }
    }

    #[test]
    fn catalog_presence_never_replaces_durable_action_authority() {
        let catalog = ready_arjun_catalog();
        for mutate in [
            |facts: &mut DurableCatalogAdmissionFactsV1| facts.exact_scope_authority = false,
            |facts: &mut DurableCatalogAdmissionFactsV1| facts.target_write_guard_passed = false,
            |facts: &mut DurableCatalogAdmissionFactsV1| facts.fuel_reserved = false,
            |facts: &mut DurableCatalogAdmissionFactsV1| facts.lease_current = false,
            |facts: &mut DurableCatalogAdmissionFactsV1| facts.jit_authorized = false,
            |facts: &mut DurableCatalogAdmissionFactsV1| facts.adapter_authority_current = false,
        ] {
            let mut facts = fully_current_catalog_facts("http_parameter_discovery");
            mutate(&mut facts);
            assert!(derive_catalog_prepared_action_admission(&catalog, &facts).is_err());
        }
    }

    #[test]
    fn catalog_admission_requires_exact_persisted_binding_and_adapter_authority() {
        let catalog = ready_arjun_catalog();
        let admitted = derive_catalog_prepared_action_admission(
            &catalog,
            &fully_current_catalog_facts("http_parameter_discovery"),
        )
        .expect("complete durable authority")
        .expect("catalog-backed route");
        assert_eq!(admitted.tool_config_id, "arjun");

        for mutate in [
            |facts: &mut DurableCatalogAdmissionFactsV1| facts.manifest_tool_config_id = None,
            |facts: &mut DurableCatalogAdmissionFactsV1| {
                facts.manifest_tool_config_id = Some("kiterunner".to_owned())
            },
            |facts: &mut DurableCatalogAdmissionFactsV1| facts.manifest_target_kind = None,
            |facts: &mut DurableCatalogAdmissionFactsV1| {
                facts.manifest_adapter_contract_digest = Some("b".repeat(64))
            },
        ] {
            let mut facts = fully_current_catalog_facts("http_parameter_discovery");
            mutate(&mut facts);
            assert!(derive_catalog_prepared_action_admission(&catalog, &facts).is_err());
        }
    }

    #[test]
    fn existing_four_capabilities_remain_on_the_trusted_get_route() {
        let catalog = load_embedded_investigation_tool_catalog().expect("embedded catalog");
        for capability_id in [
            "verify.anonymous_authenticated_differential.v1",
            "verify.directory_fingerprint.v1",
            "verify.nuclei_exact_replay.v1",
            "verify.concurrent_race_differential.v1",
        ] {
            let facts = fully_current_catalog_facts(capability_id);
            assert_eq!(
                derive_catalog_prepared_action_admission(&catalog, &facts).expect("legacy route"),
                None
            );
        }
    }
}
