use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use golish_agent_kit::db_traits::{
    EnumerationFrozenRootMemberView, RecordToolTruthShadowAssessment,
    SealToolTruthDenominatorRequest, ToolTruthDenominatorSourceRef, ToolTruthDenominatorView,
};
use golish_agent_kit::harness::tool_truth::{
    build_denominator_items, evaluate_shadow_tool_truth, DenominatorAsset, ToolTruthReceiptCoverage,
};
use golish_agent_kit::harness::StageKind;

use super::GolishDbRepoProvider;

#[derive(Debug, sqlx::FromRow)]
struct HostStageRootItem {
    denominator_id: Uuid,
    project_path_at_freeze: String,
    input_key: String,
    target_id: Uuid,
    exact_asset: String,
    target_type: String,
    technique: String,
    expected_capability: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct HostStageOutcome {
    asset: String,
    technique: String,
    outcome: String,
    source: Option<String>,
    evidence_ids: Vec<i64>,
    updated_at: chrono::DateTime<chrono::Utc>,
    seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VulnEnumerationSurfaceAuthority {
    origins: BTreeSet<String>,
    evidence_ids: Vec<i64>,
    gate_passed_at: chrono::DateTime<chrono::Utc>,
}

fn vuln_enumeration_surface_authority(
    operation_id: Uuid,
    organization_id: Uuid,
    handoffs: &[golish_db::repo::stage_handoffs::FinalSealedStageHandoffRow],
) -> anyhow::Result<VulnEnumerationSurfaceAuthority> {
    let [handoff] = handoffs else {
        anyhow::bail!("TOOL_TRUTH_VULN_ENUMERATION_SURFACE_AMBIGUOUS");
    };
    anyhow::ensure!(
        handoff.operation_id == operation_id
            && handoff.organization_id == organization_id
            && handoff.from_stage_kind == "enumeration"
            && handoff
                .coverage_watermark
                .get("kind")
                .and_then(serde_json::Value::as_str)
                == Some("information_coverage_v1")
            && handoff
                .coverage_watermark
                .get("stage")
                .and_then(serde_json::Value::as_str)
                == Some("enumeration")
            && handoff
                .coverage_watermark
                .get("organization_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
                == Some(organization_id),
        "TOOL_TRUTH_VULN_ENUMERATION_SURFACE_IDENTITY_MISMATCH"
    );
    let origins = handoff
        .coverage_watermark
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("TOOL_TRUTH_VULN_ENUMERATION_SURFACE_MISSING"))?
        .iter()
        .map(|asset| {
            asset
                .as_str()
                .and_then(golish_pentest_domain::canonical_web_origin)
                .map(|origin| origin.key)
                .ok_or_else(|| {
                    anyhow::anyhow!("TOOL_TRUTH_VULN_ENUMERATION_SURFACE_INVALID_ORIGIN")
                })
        })
        .collect::<anyhow::Result<BTreeSet<_>>>()?;
    anyhow::ensure!(
        !origins.is_empty()
            && !handoff.evidence_ids.is_empty()
            && handoff
                .evidence_ids
                .iter()
                .all(|evidence_id| *evidence_id > 0)
            && handoff
                .evidence_ids
                .windows(2)
                .all(|pair| pair[0] < pair[1]),
        "TOOL_TRUTH_VULN_ENUMERATION_SURFACE_AUTHORITY_INVALID"
    );
    Ok(VulnEnumerationSurfaceAuthority {
        origins,
        evidence_ids: handoff.evidence_ids.clone(),
        gate_passed_at: handoff.gate_passed_at,
    })
}

/// Compatibility for denominators sealed before Vuln inherited the exact
/// Enumeration Web-Origin axis. It never shrinks or rewrites the sealed root:
/// an old raw domain/IP member is either mapped to exactly one inherited
/// origin's real producer outcome, or is closed as evidence-backed N/A because
/// it is absent from the immutable executable surface. Exact-origin members
/// remain strict and must have their own terminal outcome.
fn legacy_vuln_root_surface_outcomes(
    items: &[HostStageRootItem],
    outcomes: &[HostStageOutcome],
    authority: &VulnEnumerationSurfaceAuthority,
) -> anyhow::Result<Vec<HostStageOutcome>> {
    let mut projected = Vec::new();
    for item in items {
        if host_stage_outcome_for_item("vuln_triage", item, items, outcomes).is_some()
            || golish_pentest_domain::canonical_web_origin(&item.exact_asset).is_some()
        {
            continue;
        }
        let asset_key = golish_pentest_domain::canonical_asset_key(&item.exact_asset)
            .ok_or_else(|| anyhow::anyhow!("TOOL_TRUTH_VULN_LEGACY_ASSET_INVALID"))?
            .key;
        let matching_origins = authority
            .origins
            .iter()
            .filter(|origin| {
                golish_pentest_domain::canonical_asset_key(origin)
                    .is_some_and(|candidate| candidate.key == asset_key)
            })
            .collect::<Vec<_>>();
        match matching_origins.as_slice() {
            [] => projected.push(HostStageOutcome {
                asset: item.exact_asset.clone(),
                technique: item.technique.clone(),
                outcome: "not_applicable".to_string(),
                source: Some("enumeration_final_seal:excluded_from_vuln_surface".to_string()),
                evidence_ids: authority.evidence_ids.clone(),
                updated_at: authority.gate_passed_at,
                seq: 0,
            }),
            [origin] => {
                let candidate = outcomes
                    .iter()
                    .filter(|outcome| {
                        outcome.technique == item.technique
                            && golish_pentest_domain::canonical_web_origin(&outcome.asset)
                                .is_some_and(|candidate| candidate.key == origin.as_str())
                    })
                    .max_by_key(|outcome| {
                        (outcome.updated_at, outcome.seq, outcome.asset.as_str())
                    });
                if let Some(candidate) = candidate {
                    let mut alias = candidate.clone();
                    alias.asset = item.exact_asset.clone();
                    projected.push(alias);
                }
            }
            _ => anyhow::bail!("TOOL_TRUTH_VULN_LEGACY_ASSET_AMBIGUOUS"),
        }
    }
    Ok(projected)
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExactHostStageEvidenceFact {
    evidence_asset: String,
    evidence_technique: String,
    evidence_outcome: String,
    evidence_id: i64,
    evidence_organization_id: String,
    tool_name: Option<String>,
    evidence_kind: Option<String>,
    evidence_raw_output: Option<String>,
    target_id: Uuid,
    target_organization_id: Option<Uuid>,
    target_type: String,
    target_name: String,
    target_value: String,
    target_ports: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl ExactHostStageEvidenceFact {
    fn target_bound(&self) -> golish_db::repo::audit::TargetBoundEvidenceFactRow {
        golish_db::repo::audit::TargetBoundEvidenceFactRow {
            evidence_asset: self.evidence_asset.clone(),
            evidence_technique: self.evidence_technique.clone(),
            evidence_outcome: self.evidence_outcome.clone(),
            evidence_id: self.evidence_id,
            evidence_organization_id: self.evidence_organization_id.clone(),
            tool_name: self.tool_name.clone(),
            evidence_kind: self.evidence_kind.clone(),
            evidence_raw_output: self.evidence_raw_output.clone(),
            target_id: self.target_id,
            target_organization_id: self.target_organization_id,
            target_type: self.target_type.clone(),
            target_name: self.target_name.clone(),
            target_value: self.target_value.clone(),
            target_ports: self.target_ports.clone(),
        }
    }
}

fn strengthen_host_stage_outcomes_from_exact_evidence(
    stage_kind: &str,
    organization_id: Uuid,
    evidence: &[ExactHostStageEvidenceFact],
    outcomes: &mut Vec<HostStageOutcome>,
) {
    if stage_kind != "external_attack_surface" {
        return;
    }
    let metadata = evidence
        .iter()
        .map(|row| (row.evidence_id, (row.created_at, row.tool_name.clone())))
        .collect::<BTreeMap<_, _>>();
    let facts = super::evidence::eas_target_bound_evidence_facts(
        organization_id,
        evidence
            .iter()
            .map(ExactHostStageEvidenceFact::target_bound),
    );
    for (asset, technique, outcome, evidence_id) in facts {
        let Some((created_at, source)) = metadata.get(&evidence_id) else {
            continue;
        };
        outcomes.push(HostStageOutcome {
            asset: asset.clone(),
            technique: technique.clone(),
            outcome: outcome.clone(),
            source: source.clone(),
            evidence_ids: vec![evidence_id],
            updated_at: *created_at,
            seq: evidence_id,
        });
        if technique == golish_db::repo::coverage_truth::TECH_EAS_PORT
            && outcome == "blocked"
            && source.as_deref() == Some("eas_discover_ports")
        {
            // Service fingerprinting has no executable input when the exact
            // exhaustive-port producer is policy-blocked. Preserve that
            // inconclusive residual as evidence-backed blocked truth rather
            // than pretending the service producer ran or rescanning.
            outcomes.push(HostStageOutcome {
                asset,
                technique: golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP.to_string(),
                outcome: "blocked".to_string(),
                source: Some("eas_discover_ports:blocked_prerequisite".to_string()),
                evidence_ids: vec![evidence_id],
                updated_at: *created_at,
                seq: evidence_id,
            });
        }
    }
}

fn host_stage_terminal_observation(outcome: Option<&HostStageOutcome>) -> Option<&'static str> {
    let outcome = outcome.filter(|row| !row.evidence_ids.is_empty())?;
    match outcome.outcome.as_str() {
        "found" => Some("found"),
        // A producer-owned blocked result is terminal execution truth only when
        // it carries current-run evidence.  The receipt records that the
        // producer returned no positive match, while typed_landing and the raw
        // witness retain the blocked outcome and its inconclusive residual.
        "empty" | "not_applicable" | "blocked" => Some("no_match"),
        _ => None,
    }
}

fn host_stage_outcome_is_fresh(
    outcome: &HostStageOutcome,
    stage_started_at: chrono::DateTime<chrono::Utc>,
) -> bool {
    outcome.updated_at >= stage_started_at
}

fn host_stage_outcome_for_item<'a>(
    stage_kind: &str,
    item: &HostStageRootItem,
    items: &[HostStageRootItem],
    outcomes: &'a [HostStageOutcome],
) -> Option<&'a HostStageOutcome> {
    let latest = |candidates: Vec<&'a HostStageOutcome>| {
        candidates
            .into_iter()
            .max_by_key(|outcome| (outcome.updated_at, outcome.seq, outcome.asset.as_str()))
    };
    let exact = outcomes
        .iter()
        .filter(|outcome| outcome.technique == item.technique && outcome.asset == item.exact_asset)
        .collect::<Vec<_>>();
    if !exact.is_empty() {
        return latest(exact);
    }

    if matches!(stage_kind, "enumeration" | "vuln_triage") {
        if let Some(item_origin) = golish_pentest_domain::canonical_web_origin(&item.exact_asset) {
            let canonical = outcomes
                .iter()
                .filter(|outcome| {
                    outcome.technique == item.technique
                        && golish_pentest_domain::canonical_web_origin(&outcome.asset)
                            .is_some_and(|origin| origin.key == item_origin.key)
                })
                .collect::<Vec<_>>();
            if !canonical.is_empty() {
                return latest(canonical);
            }
        }
    }

    if stage_kind != "vuln_triage" || !item.target_type.eq_ignore_ascii_case("domain") {
        return None;
    }
    let domain_key = golish_pentest_domain::canonical_asset_key(&item.exact_asset)?.key;
    let sibling_origins = items
        .iter()
        .filter(|sibling| sibling.target_type.eq_ignore_ascii_case("url"))
        .filter(|sibling| {
            golish_pentest_domain::canonical_asset_key(&sibling.exact_asset)
                .is_some_and(|key| key.key == domain_key)
        })
        .filter_map(|sibling| {
            golish_pentest_domain::canonical_web_origin(&sibling.exact_asset)
                .map(|origin| origin.key)
        })
        .collect::<BTreeSet<_>>();
    if sibling_origins.len() != 1 {
        return None;
    }
    let sibling_origin = sibling_origins.first()?;
    latest(
        outcomes
            .iter()
            .filter(|outcome| {
                outcome.technique == item.technique
                    && golish_pentest_domain::canonical_web_origin(&outcome.asset)
                        .is_some_and(|origin| origin.key == *sibling_origin)
            })
            .collect(),
    )
}

pub(super) fn stable_denominator_seal_request(stage_execution_id: Uuid, source_id: Uuid) -> Uuid {
    Uuid::new_v5(&stage_execution_id, source_id.as_bytes())
}

impl GolishDbRepoProvider {
    pub(super) async fn tool_truth_finalize_stage_root_impl(
        &self,
        mut request: golish_agent_kit::db_traits::FinalizeStageToolTruthRequest,
    ) -> anyhow::Result<golish_agent_kit::db_traits::StageToolTruthCloseoutView> {
        anyhow::ensure!(
            matches!(
                request.stage_kind.as_str(),
                "external_attack_surface" | "enumeration" | "vuln_triage"
            ),
            "TOOL_TRUTH_HOST_STAGE_KIND_UNSUPPORTED"
        );
        request
            .outcome_run_ids
            .push(request.operation_id.to_string());
        request.outcome_run_ids.sort();
        request.outcome_run_ids.dedup();
        anyhow::ensure!(
            !request.outcome_run_ids.is_empty()
                && request
                    .outcome_run_ids
                    .iter()
                    .all(|run_id| !run_id.trim().is_empty()),
            "TOOL_TRUTH_HOST_STAGE_OUTCOME_RUN_INVALID"
        );

        let items = sqlx::query_as::<_, HostStageRootItem>(
            r#"SELECT denominator.id AS denominator_id,
                      denominator.project_path_at_freeze,
                      item.input_key,item.target_id,item.exact_asset,
                      target.target_type::text AS target_type,item.technique,
                      item.expected_capability
                 FROM coverage_denominators denominator
                 JOIN tool_truth_execution_authorities authority
                   ON authority.id=denominator.execution_authority_id
                 JOIN coverage_denominator_items item
                   ON item.denominator_id=denominator.id
                 JOIN targets target ON target.id=item.target_id
                WHERE denominator.operation_id=$1
                  AND denominator.organization_id=$2
                  AND denominator.stage_execution_id=$3
                  AND denominator.stage_kind=$4
                  AND denominator.denominator_kind='root'
                  AND denominator.sealed_at IS NOT NULL
                  AND authority.execution_owner_kind='host_stage'
                  AND authority.execution_source_kind='stage_unit'
                  AND authority.stage_run_unit_id=$5
                ORDER BY item.ordinal"#,
        )
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(request.stage_execution_id)
        .bind(&request.stage_kind)
        .bind(request.stage_run_unit_id)
        .fetch_all(self.pool.as_ref())
        .await?;
        anyhow::ensure!(!items.is_empty(), "TOOL_TRUTH_HOST_STAGE_ROOT_MISSING");
        let denominator_ids = items
            .iter()
            .map(|item| item.denominator_id)
            .collect::<BTreeSet<_>>();
        let project_paths = items
            .iter()
            .map(|item| item.project_path_at_freeze.as_str())
            .collect::<BTreeSet<_>>();
        anyhow::ensure!(
            denominator_ids.len() == 1 && project_paths.len() == 1,
            "TOOL_TRUTH_HOST_STAGE_ROOT_AMBIGUOUS"
        );
        let denominator_id = *denominator_ids
            .first()
            .ok_or_else(|| anyhow::anyhow!("TOOL_TRUTH_HOST_STAGE_ROOT_MISSING"))?;
        let project_path_at_freeze = project_paths
            .first()
            .ok_or_else(|| anyhow::anyhow!("TOOL_TRUTH_HOST_STAGE_PROJECT_MISSING"))?
            .to_string();
        let project_root = std::fs::canonicalize(&project_path_at_freeze)?;
        let assets = items
            .iter()
            .map(|item| item.exact_asset.clone())
            .collect::<Vec<_>>();
        let target_types = items
            .iter()
            .map(|item| item.target_type.clone())
            .collect::<Vec<_>>();
        let found = golish_db::repo::coverage_truth::coverage_truth_facts(
            self.pool.as_ref(),
            Some(request.organization_id),
            &assets,
            &target_types,
            Some(request.stage_started_at),
        )
        .await?
        .into_iter()
        .map(|(asset, technique)| (asset, technique.to_string()))
        .collect::<BTreeSet<_>>();
        let mut outcomes = sqlx::query_as::<_, HostStageOutcome>(
            r#"SELECT DISTINCT ON (asset,technique)
                      asset,technique,outcome,source,evidence_ids,updated_at,seq
                 FROM technique_outcomes
                WHERE organization_id=$1
                  AND run_id=ANY($2)
                ORDER BY asset,technique,updated_at DESC,seq DESC"#,
        )
        .bind(request.organization_id)
        .bind(&request.outcome_run_ids)
        .fetch_all(self.pool.as_ref())
        .await?
        .into_iter()
        .filter(|outcome| host_stage_outcome_is_fresh(outcome, request.stage_started_at))
        .collect::<Vec<_>>();
        let target_ids = items.iter().map(|item| item.target_id).collect::<Vec<_>>();
        let exact_evidence = sqlx::query_as::<_, ExactHostStageEvidenceFact>(
            r#"SELECT evidence.evidence_asset,evidence.evidence_technique,
                      evidence.evidence_outcome,evidence.id AS evidence_id,
                      evidence.detail->>'organization_id' AS evidence_organization_id,
                      evidence.tool_name,evidence.detail->>'kind' AS evidence_kind,
                      evidence.detail->>'raw_output' AS evidence_raw_output,
                      target.id AS target_id,target.organization_id AS target_organization_id,
                      target.target_type::text AS target_type,target.name AS target_name,
                      target.value AS target_value,COALESCE(target.ports,'[]'::jsonb) AS target_ports,
                      evidence.created_at
                 FROM audit_log evidence
                 JOIN evidence_classifications classification
                   ON classification.evidence_audit_id=evidence.id
                  AND classification.valid_to IS NULL
                  AND classification.classification='in_scope'
                  AND classification.producing_stage_run_id=$6::uuid
                 JOIN targets target
                   ON target.id=evidence.target_id
                  AND target.organization_id=$5::uuid
                  AND target.scope::text='in'
                  AND target.project_path=$2
                 JOIN stage_worker_runs worker
                   ON worker.id::text=(evidence.detail #>> '{tool_truth_producer,worker_run_id}')
                  AND worker.operation_id=$1::uuid
                  AND worker.stage_execution_id=$6::uuid
                  AND worker.stage_run_unit_id=$7::uuid
                  AND worker.organization_id=$5::uuid
                 JOIN tool_calls tool
                   ON tool.id::text=(evidence.detail #>> '{tool_truth_producer,source_tool_call_id}')
                  AND tool.worker_run_id=worker.id
                  AND tool.operation_id=worker.operation_id
                  AND tool.stage_execution_id=worker.stage_execution_id
                  AND tool.stage_run_unit_id=worker.stage_run_unit_id
                  AND tool.organization_id=worker.organization_id
                  AND tool.status='finished'
                WHERE evidence.audit_role='evidence'
                  AND evidence.run_id=$1::uuid
                  AND evidence.project_path=$2
                  AND evidence.target_id=ANY($3)
                  AND evidence.created_at >= $4
                  AND evidence.detail->>'organization_id'=($5::uuid)::text
                  AND evidence.detail #>> '{tool_truth_producer,stage_execution_id}'=($6::uuid)::text
                  AND evidence.detail #>> '{tool_truth_producer,stage_run_unit_id}'=($7::uuid)::text
                  AND evidence.detail #>> '{tool_truth_producer,organization_id}'=($5::uuid)::text
                  AND evidence.detail #>> '{tool_truth_producer,producer_tool_name}'=tool.name
                  AND evidence.tool_name=tool.name
                  AND evidence.evidence_asset IS NOT NULL
                  AND evidence.evidence_technique IS NOT NULL
                  AND evidence.evidence_outcome IS NOT NULL
                ORDER BY evidence.id"#,
        )
        .bind(request.operation_id)
        .bind(&project_path_at_freeze)
        .bind(&target_ids)
        .bind(request.stage_started_at)
        .bind(request.organization_id)
        .bind(request.stage_execution_id)
        .bind(request.stage_run_unit_id)
        .fetch_all(self.pool.as_ref())
        .await?;
        strengthen_host_stage_outcomes_from_exact_evidence(
            &request.stage_kind,
            request.organization_id,
            &exact_evidence,
            &mut outcomes,
        );

        if request.stage_kind == "vuln_triage" {
            let handoffs = golish_db::repo::stage_handoffs::list_latest_final_sealed_for_sources(
                self.pool.as_ref(),
                request.operation_id,
                request.organization_id,
                &["enumeration".to_string()],
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let authority = vuln_enumeration_surface_authority(
                request.operation_id,
                request.organization_id,
                &handoffs,
            )?;
            outcomes.extend(legacy_vuln_root_surface_outcomes(
                &items, &outcomes, &authority,
            )?);
        }

        let resolved_outcomes = items
            .iter()
            .map(|item| host_stage_outcome_for_item(&request.stage_kind, item, &items, &outcomes))
            .collect::<Vec<_>>();

        let mut observations = Vec::with_capacity(items.len());
        let mut witness_items = Vec::with_capacity(items.len());
        let mut gaps = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let outcome = resolved_outcomes[index];
            let observation_state =
                if found.contains(&(item.exact_asset.clone(), item.technique.clone())) {
                    "found"
                } else {
                    host_stage_terminal_observation(outcome).unwrap_or("indeterminate")
                };
            if observation_state == "indeterminate" {
                gaps.push(format!(
                    "{}:{}:{}",
                    item.exact_asset, item.technique, item.expected_capability
                ));
            }
            observations.push(
                golish_db::repo::capability_execution_receipts::TargetIntelInputObservation {
                    input_key: item.input_key.clone(),
                    technique: item.technique.clone(),
                    observation_state: observation_state.to_string(),
                },
            );
            witness_items.push(serde_json::json!({
                "input_key": item.input_key,
                "target_id": item.target_id,
                "exact_asset": item.exact_asset,
                "target_type": item.target_type,
                "technique": item.technique,
                "expected_capability": item.expected_capability,
                "observation_state": observation_state,
                "producer_outcome": outcome.map(|row| row.outcome.as_str()),
                "producer_source": outcome.and_then(|row| row.source.as_deref()),
                "evidence_ids": outcome.map(|row| row.evidence_ids.as_slice()).unwrap_or(&[]),
            }));
        }
        anyhow::ensure!(
            gaps.is_empty(),
            "TOOL_TRUTH_HOST_STAGE_INCOMPLETE: {}",
            gaps.join(",")
        );

        let mut receipt_ids = Vec::new();
        let mut by_capability = BTreeMap::<String, Vec<usize>>::new();
        for (index, item) in items.iter().enumerate() {
            by_capability
                .entry(item.expected_capability.clone())
                .or_default()
                .push(index);
        }
        for (capability, indexes) in by_capability {
            let blocked_input_count = indexes
                .iter()
                .filter(|index| {
                    resolved_outcomes[**index].is_some_and(|outcome| {
                        outcome.outcome == "blocked" && !outcome.evidence_ids.is_empty()
                    })
                })
                .count();
            let policy = golish_db::repo::capability_execution_receipts::seal_host_stage_reconciliation_policy(
                self.pool.as_ref(),
                &golish_db::repo::capability_execution_receipts::SealHostStageReconciliationPolicy {
                    denominator_id,
                    capability: capability.clone(),
                },
            )
            .await?;
            let receipt_id = Uuid::new_v5(
                &denominator_id,
                format!("host-stage-reconciliation:{capability}:v1").as_bytes(),
            );
            let receipt = match golish_db::repo::capability_execution_receipts::begin_managed_claim(
                self.pool.as_ref(),
                &golish_db::repo::capability_execution_receipts::BeginManagedCapabilityReceipt {
                    id: receipt_id,
                    denominator_id,
                    capability: capability.clone(),
                    attempt_ordinal: 1,
                    destination_policy_id: policy.id,
                },
            )
            .await?
            {
                golish_db::repo::capability_execution_receipts::ManagedReceiptBeginOutcome::Created(row)
                | golish_db::repo::capability_execution_receipts::ManagedReceiptBeginOutcome::InFlight(row) => row,
                golish_db::repo::capability_execution_receipts::ManagedReceiptBeginOutcome::TerminalReplay(row) => {
                    anyhow::ensure!(
                        row.reconciliation_state == "consistent"
                            && row.coverage_extent == "complete",
                        "TOOL_TRUTH_HOST_STAGE_RECEIPT_REPLAY_INVALID"
                    );
                    receipt_ids.push(row.id);
                    continue;
                }
            };
            let capability_witness = serde_json::to_vec(&serde_json::json!({
                "schema": "tool_truth_host_stage_reconciliation_v1",
                "operation_id": request.operation_id,
                "organization_id": request.organization_id,
                "stage_execution_id": request.stage_execution_id,
                "stage_run_unit_id": request.stage_run_unit_id,
                "stage_kind": request.stage_kind,
                "denominator_id": denominator_id,
                "capability": capability,
                "items": indexes
                    .iter()
                    .map(|index| witness_items[*index].clone())
                    .collect::<Vec<_>>(),
            }))?;
            let byte_count = i64::try_from(capability_witness.len())?;
            let raw_witness = super::recon::seal_tool_truth_witness(
                &project_root,
                request.operation_id,
                receipt.id,
                &capability_witness,
                byte_count,
                false,
            )?;
            let input_observations = indexes
                .iter()
                .map(|index| observations[*index].clone())
                .collect::<Vec<_>>();
            let normalized_record_count = i64::try_from(
                input_observations
                    .iter()
                    .filter(|observation| observation.observation_state == "found")
                    .count(),
            )?;
            let finalized =
                golish_db::repo::capability_execution_receipts::finalize_host_stage_receipt(
                    self.pool.as_ref(),
                    &golish_db::repo::capability_execution_receipts::FinalizeTargetIntelReceipt {
                        receipt_id: receipt.id,
                        expected_row_version: receipt.row_version,
                        attempt_fence: None,
                        raw_witness: raw_witness.clone(),
                        network_hops: Vec::new(),
                        request_count: 0,
                        response_byte_count: raw_witness.stored_byte_count,
                        wall_clock_ms: 0,
                        retry_count: 0,
                        parser_complete: true,
                        normalized_record_count,
                        input_observations,
                        typed_landing: serde_json::json!({
                        "schema": "tool_truth_host_stage_reconciliation_v1",
                        "stage_kind": request.stage_kind,
                        "denominator_id": denominator_id,
                        "capability": capability,
                            "state": "reconciled",
                            "blocked_input_count": blocked_input_count,
                            "security_interpretation": if blocked_input_count > 0 {
                                "inconclusive"
                            } else {
                                "not_assessed"
                            },
                        }),
                        failure_reason_code: None,
                    },
                )
                .await?;
            anyhow::ensure!(
                finalized.reconciliation_state == "consistent"
                    && finalized.coverage_extent == "complete",
                "TOOL_TRUTH_HOST_STAGE_RECEIPT_NOT_FRESH"
            );
            receipt_ids.push(finalized.id);
        }
        receipt_ids.sort();
        Ok(golish_agent_kit::db_traits::StageToolTruthCloseoutView {
            denominator_id,
            expected_input_count: i64::try_from(items.len())?,
            finalized_receipt_count: i64::try_from(receipt_ids.len())?,
            receipt_ids,
        })
    }

    pub(super) async fn tool_truth_seal_denominator_impl(
        &self,
        request: SealToolTruthDenominatorRequest,
    ) -> anyhow::Result<ToolTruthDenominatorView> {
        let source = match request.source {
            ToolTruthDenominatorSourceRef::StageAssetWave {
                stage_asset_wave_id,
            } => {
                golish_db::repo::capability_execution_receipts::DenominatorSourceRef::StageAssetWave(
                    stage_asset_wave_id,
                )
            }
            ToolTruthDenominatorSourceRef::StageTeamUnit { stage_run_unit_id } => {
                golish_db::repo::capability_execution_receipts::DenominatorSourceRef::StageTeamUnit(
                    stage_run_unit_id,
                )
            }
        };
        let row = golish_db::repo::capability_execution_receipts::seal_source_denominator(
            &self.pool,
            &golish_db::repo::capability_execution_receipts::SealSourceDenominator {
                stable_seal_request_id: request.stable_seal_request_id,
                stage_execution_id: request.stage_execution_id,
                source,
            },
            |stage, locked| {
                let stage = StageKind::try_parse(stage)
                    .ok_or_else(|| anyhow::anyhow!("TOOL_TRUTH_STAGE_KIND_INVALID"))?;
                let assets = locked
                    .iter()
                    .map(|asset| DenominatorAsset {
                        target_id: asset.target_id,
                        exact_asset: asset.exact_asset.clone(),
                        asset_type: asset.asset_type.clone(),
                        web_capable: asset.web_capable,
                    })
                    .collect::<Vec<_>>();
                build_denominator_items(stage, &assets)
                    .map(|items| {
                        items
                            .into_iter()
                            .map(|item| {
                                golish_db::repo::capability_execution_receipts::CompiledDenominatorItem {
                                    input_key: item.input_key,
                                    target_id: item.target_id,
                                    exact_asset: item.exact_asset,
                                    technique: item.technique,
                                    expected_capability: item.expected_capability,
                                }
                            })
                            .collect()
                    })
                    .map_err(Into::into)
            },
        )
        .await?;
        Ok(ToolTruthDenominatorView {
            id: row.id,
            execution_authority_id: row.execution_authority_id,
            input_manifest_hash: row.input_manifest_hash,
            member_count: row
                .member_count
                .ok_or_else(|| anyhow::anyhow!("TOOL_TRUTH_DENOMINATOR_UNSEALED"))?,
            denominator_hash: row.denominator_hash,
        })
    }

    pub(super) async fn enumeration_frozen_root_members_impl(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage_execution_id: Uuid,
        stage_run_unit_id: Uuid,
    ) -> anyhow::Result<Vec<EnumerationFrozenRootMemberView>> {
        let stable_seal_request_id =
            stable_denominator_seal_request(stage_execution_id, stage_run_unit_id);
        let mut tx = self.pool.begin().await?;
        let (denominator_id, member_count) = sqlx::query_as::<_, (Uuid, Option<i64>)>(
            r#"SELECT denominator.id,denominator.member_count
                  FROM coverage_denominators denominator
                  JOIN tool_truth_execution_authorities authority
                    ON authority.id=denominator.execution_authority_id
                  JOIN stage_run_units unit
                    ON unit.id=$4
                   AND unit.operation_id=denominator.operation_id
                   AND unit.stage_execution_id=denominator.stage_execution_id
                   AND unit.scope_snapshot_id=denominator.scope_snapshot_id
                   AND unit.organization_id=denominator.organization_id
                   AND unit.stage_kind=denominator.stage_kind
                 WHERE denominator.operation_id=$1
                   AND denominator.organization_id=$2
                   AND denominator.stage_execution_id=$3
                   AND denominator.stage_kind='enumeration'
                   AND denominator.stable_seal_request_id=$5
                   AND denominator.denominator_kind='root'
                   AND denominator.sealed_at IS NOT NULL
                   AND authority.stage_run_unit_id=$4
                   AND authority.execution_owner_kind='host_stage'
                 FOR SHARE OF denominator,authority,unit"#,
        )
        .bind(operation_id)
        .bind(organization_id)
        .bind(stage_execution_id)
        .bind(stage_run_unit_id)
        .bind(stable_seal_request_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("ENUMERATION_FROZEN_ROOT_MISSING"))?;
        let rows = sqlx::query_as::<_, (Option<Uuid>, String, String, String)>(
            r#"SELECT target_id,exact_asset,technique,expected_capability
                  FROM coverage_denominator_items
                 WHERE denominator_id=$1
                 ORDER BY target_id,exact_asset,technique
                 FOR SHARE"#,
        )
        .bind(denominator_id)
        .fetch_all(&mut *tx)
        .await?;
        anyhow::ensure!(
            member_count == i64::try_from(rows.len()).ok(),
            "ENUMERATION_FROZEN_ROOT_MEMBER_COUNT_DRIFT"
        );

        let required_axes = [
            "GOLISH-ENUM-DIR",
            "GOLISH-ENUM-JS",
            "GOLISH-ENUM-JSAPI",
            "GOLISH-ENUM-PARAM",
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
        let mut axes_by_subject =
            std::collections::BTreeMap::<(Uuid, String), std::collections::BTreeSet<String>>::new();
        let mut members = Vec::with_capacity(rows.len());
        for (target_id, exact_origin, technique, expected_capability) in rows {
            let target_id = target_id
                .filter(|target_id| !target_id.is_nil())
                .ok_or_else(|| anyhow::anyhow!("ENUMERATION_FROZEN_ROOT_TARGET_MISSING"))?;
            let canonical = golish_pentest_domain::canonical_web_origin(&exact_origin)
                .filter(|origin| origin.key == exact_origin)
                .ok_or_else(|| anyhow::anyhow!("ENUMERATION_FROZEN_ROOT_ORIGIN_INVALID"))?;
            anyhow::ensure!(
                required_axes.contains(technique.as_str())
                    && !expected_capability.trim().is_empty(),
                "ENUMERATION_FROZEN_ROOT_AXIS_INVALID"
            );
            let inserted = axes_by_subject
                .entry((target_id, canonical.key.clone()))
                .or_default()
                .insert(technique.clone());
            anyhow::ensure!(inserted, "ENUMERATION_FROZEN_ROOT_DUPLICATE_AXIS");
            members.push(EnumerationFrozenRootMemberView {
                target_id,
                exact_origin: canonical.key,
                technique,
                expected_capability,
            });
        }
        anyhow::ensure!(
            !axes_by_subject.is_empty()
                && axes_by_subject.values().all(|axes| {
                    axes.iter()
                        .map(String::as_str)
                        .collect::<std::collections::BTreeSet<_>>()
                        == required_axes
                }),
            "ENUMERATION_FROZEN_ROOT_AXIS_CLOSURE_MISMATCH"
        );
        tx.commit().await?;
        Ok(members)
    }

    pub(super) async fn seal_wave_before_dispatch(
        &self,
        stage_execution_id: Uuid,
        operation_id: Uuid,
        stage_asset_wave_id: Uuid,
    ) -> anyhow::Result<()> {
        let contract =
            golish_db::repo::operation_state::get_tool_truth_contract(&self.pool, operation_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("TOOL_TRUTH_OPERATION_CONTRACT_MISSING"))?;
        if !contract.writes_receipts() {
            return Ok(());
        }
        if stage_execution_id.is_nil() {
            anyhow::bail!("TOOL_TRUTH_STAGE_EXECUTION_MISSING");
        }
        let denominator = self
            .tool_truth_seal_denominator_impl(SealToolTruthDenominatorRequest {
                stable_seal_request_id: stable_denominator_seal_request(
                    stage_execution_id,
                    stage_asset_wave_id,
                ),
                stage_execution_id,
                source: ToolTruthDenominatorSourceRef::StageAssetWave {
                    stage_asset_wave_id,
                },
            })
            .await?;
        tracing::debug!(
            denominator_id = %denominator.id,
            wave_id = %stage_asset_wave_id,
            "tool-truth denominator sealed before provider dispatch"
        );
        Ok(())
    }

    pub(super) async fn tool_truth_record_shadow_assessment_impl(
        &self,
        request: RecordToolTruthShadowAssessment,
    ) -> anyhow::Result<()> {
        let stage_execution_ids = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT id FROM stage_runs
                WHERE operation_id=$1 AND stage_kind=$2 AND status='started'
                ORDER BY started_at,id FOR SHARE"#,
        )
        .bind(request.operation_id)
        .bind(&request.stage_kind)
        .fetch_all(&*self.pool)
        .await?;
        let [stage_execution_id] = stage_execution_ids.as_slice() else {
            anyhow::bail!("TOOL_TRUTH_ACTIVE_STAGE_EXECUTION_AMBIGUOUS");
        };

        let denominator = sqlx::query_as::<_, (Uuid, String, Uuid, String, Uuid, String, Uuid)>(
            r#"SELECT d.id,d.denominator_hash,a.id,a.authority_hash,
                      a.project_scope_id,a.project_path_at_freeze,a.scope_snapshot_id
                 FROM coverage_denominators d
                 JOIN tool_truth_execution_authorities a ON a.id=d.execution_authority_id
                WHERE d.operation_id=$1 AND d.organization_id=$2
                  AND d.stage_execution_id=$3 AND d.stage_kind=$4
                  AND d.denominator_kind='root' AND d.sealed_at IS NOT NULL
                  AND ($5::uuid IS NULL OR EXISTS (
                      SELECT 1 FROM tool_truth_stage_wave_execution_bindings b
                       WHERE b.id=a.stage_wave_binding_id AND b.stage_asset_wave_id=$5
                  ))
                ORDER BY d.created_at DESC,d.id DESC LIMIT 1"#,
        )
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(stage_execution_id)
        .bind(&request.stage_kind)
        .bind(request.stage_asset_wave_id)
        .fetch_optional(&*self.pool)
        .await?;

        let (authority_id, authority_hash, project_scope_id, project_path, snapshot_id) =
            if let Some(denominator) = &denominator {
                (
                    denominator.2,
                    denominator.3.clone(),
                    denominator.4,
                    denominator.5.clone(),
                    denominator.6,
                )
            } else {
                let scope = sqlx::query_as::<_, (Uuid, String, Uuid)>(
                    r#"SELECT project_scope_id,project_path_at_freeze,id
                         FROM operation_org_scope_snapshots s
                        WHERE s.operation_id=$1 AND s.sealed_at IS NOT NULL
                          AND EXISTS (
                              SELECT 1 FROM operation_org_scope_units u
                               WHERE u.snapshot_id=s.id AND u.organization_id=$2
                          ) FOR SHARE"#,
                )
                .bind(request.operation_id)
                .bind(request.organization_id)
                .fetch_one(&*self.pool)
                .await?;
                let stable = Uuid::new_v5(
                    stage_execution_id,
                    format!(
                        "missing-denominator:{}:{}",
                        request.organization_id, request.stage_kind
                    )
                    .as_bytes(),
                );
                sqlx::query(
                    r#"INSERT INTO tool_truth_execution_authorities(
                           id,stable_authority_request_id,operation_id,project_scope_id,
                           project_path_at_freeze,scope_snapshot_id,organization_id,
                           stage_execution_id,stage_kind,execution_source_kind,
                           execution_owner_kind,authority_hash
                       ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'stage_execution','host_stage',$10)
                       ON CONFLICT(operation_id,stable_authority_request_id) DO NOTHING"#,
                )
                .bind(Uuid::new_v4())
                .bind(stable)
                .bind(request.operation_id)
                .bind(scope.0)
                .bind(&scope.1)
                .bind(scope.2)
                .bind(request.organization_id)
                .bind(stage_execution_id)
                .bind(&request.stage_kind)
                .bind(format!("sha256:{}", "0".repeat(64)))
                .execute(&*self.pool)
                .await?;
                let authority = sqlx::query_as::<_, (Uuid, String)>(
                    r#"SELECT id,authority_hash FROM tool_truth_execution_authorities
                        WHERE operation_id=$1 AND stable_authority_request_id=$2"#,
                )
                .bind(request.operation_id)
                .bind(stable)
                .fetch_one(&*self.pool)
                .await?;
                (authority.0, authority.1, scope.0, scope.1, scope.2)
            };

        let coverage = if let Some((denominator_id, _, _, _, _, _, _)) = denominator.as_ref() {
            let counts = sqlx::query_as::<_, (i64, i64, i64)>(
                r#"SELECT count(*)::bigint,
                          count(*) FILTER (WHERE EXISTS (
                              SELECT 1 FROM capability_execution_receipt_inputs i
                              JOIN capability_execution_receipts r ON r.id=i.receipt_id
                               WHERE i.denominator_item_id=di.id
                                 AND i.denominator_id=di.denominator_id
                                 AND i.sealed_at IS NOT NULL
                                 AND i.coverage_extent='complete'
                                 AND i.landing_state='committed'
                                 AND r.finalized_at IS NOT NULL
                                 AND r.reconciliation_state='consistent'
                                 AND r.current_semantic_reconciliation_id IS NOT NULL
                          ))::bigint,
                          count(*) FILTER (WHERE EXISTS (
                              SELECT 1 FROM capability_execution_receipt_inputs i
                               WHERE i.denominator_item_id=di.id
                                 AND i.denominator_id=di.denominator_id
                                 AND i.sealed_at IS NOT NULL
                                 AND i.coverage_extent IN ('partial','sampled','template_only')
                          ))::bigint
                     FROM coverage_denominator_items di WHERE di.denominator_id=$1"#,
            )
            .bind(denominator_id)
            .fetch_one(&*self.pool)
            .await?;
            Some(ToolTruthReceiptCoverage {
                expected: usize::try_from(counts.0)?,
                terminal: usize::try_from(counts.1)?,
                degraded: usize::try_from(counts.2)?,
            })
        } else {
            None
        };
        let assessment = evaluate_shadow_tool_truth(request.legacy_allowed, coverage, &[]);

        let authority_set_id = if let Some((denominator_id, denominator_hash, ..)) = &denominator {
            let stable = Uuid::new_v5(
                stage_execution_id,
                format!(
                    "authority-set:{denominator_hash}:{}:{}",
                    coverage.map_or(0, |value| value.terminal),
                    coverage.map_or(0, |value| value.degraded)
                )
                .as_bytes(),
            );
            let id = Uuid::new_v4();
            sqlx::query(
                r#"INSERT INTO tool_truth_authority_set_seals(
                       id,stable_consumer_request_id,execution_authority_id,denominator_id,
                       denominator_hash,consumer_kind,graph_hash,semantic_hash,freshness_hash
                   ) VALUES($1,$2,$3,$4,$5,'org_gate_shadow',
                       tool_truth_sha256($6),tool_truth_sha256($7),tool_truth_sha256($8))
                   ON CONFLICT(execution_authority_id,stable_consumer_request_id) DO NOTHING"#,
            )
            .bind(id)
            .bind(stable)
            .bind(authority_id)
            .bind(denominator_id)
            .bind(denominator_hash)
            .bind(format!("graph:{denominator_hash}"))
            .bind(format!("semantic:{denominator_hash}"))
            .bind(format!("freshness:{denominator_hash}"))
            .execute(&*self.pool)
            .await?;
            let id = sqlx::query_scalar::<_, Uuid>(
                r#"SELECT id FROM tool_truth_authority_set_seals
                    WHERE execution_authority_id=$1 AND stable_consumer_request_id=$2"#,
            )
            .bind(authority_id)
            .bind(stable)
            .fetch_one(&*self.pool)
            .await?;
            let sealed: bool = sqlx::query_scalar(
                "SELECT sealed_at IS NOT NULL FROM tool_truth_authority_set_seals WHERE id=$1",
            )
            .bind(id)
            .fetch_one(&*self.pool)
            .await?;
            if !sealed {
                sqlx::query(
                    "UPDATE tool_truth_authority_set_seals SET sealed_at=statement_timestamp() WHERE id=$1",
                )
                .bind(id)
                .execute(&*self.pool)
                .await?;
            }
            Some(id)
        } else {
            None
        };

        let basis = if denominator.is_some() {
            "authority_set"
        } else {
            "missing_denominator"
        };
        let expected = coverage.map_or(0_i64, |value| value.expected as i64);
        let terminal = coverage.map_or(0_i64, |value| value.terminal as i64);
        let degraded = coverage.map_or(0_i64, |value| value.degraded as i64);
        let stable_gate_request_id = Uuid::new_v5(
            stage_execution_id,
            format!(
                "gate:{basis}:{}:{expected}:{terminal}:{degraded}:{}",
                request.organization_id, request.legacy_allowed
            )
            .as_bytes(),
        );
        let denominator_id = denominator.as_ref().map(|value| value.0);
        let residual = if denominator_id.is_none() {
            serde_json::json!({"reason_code": "TOOL_TRUTH_DENOMINATOR_MISSING"})
        } else {
            serde_json::json!({"missing_item_count": expected-terminal})
        };
        sqlx::query(
            r#"INSERT INTO tool_truth_gate_assessments(
                   id,stable_gate_request_id,operation_id,project_scope_id,
                   project_path_at_freeze,scope_snapshot_id,organization_id,
                   stage_execution_id,stage_kind,execution_authority_id,
                   execution_authority_hash,assessment_basis_kind,denominator_id,
                   authority_set_id,legacy_allowed,control_decision,coverage_grade,
                   divergence,expected_item_count,terminal_item_count,degraded_item_count,
                   residual,assessment_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                        $18,$19,$20,$21,$22,
                        tool_truth_sha256(jsonb_build_object(
                            'stable_gate_request_id',$2,'execution_authority_id',$10,
                            'basis',$12,'denominator_id',$13,'authority_set_id',$14,
                            'legacy_allowed',$15,'control_decision',$16,'coverage_grade',$17,
                            'expected',$19,'terminal',$20,'degraded',$21,'residual',$22
                        )::text))
               ON CONFLICT(operation_id,stable_gate_request_id) DO NOTHING"#,
        )
        .bind(Uuid::new_v4())
        .bind(stable_gate_request_id)
        .bind(request.operation_id)
        .bind(project_scope_id)
        .bind(&project_path)
        .bind(snapshot_id)
        .bind(request.organization_id)
        .bind(stage_execution_id)
        .bind(&request.stage_kind)
        .bind(authority_id)
        .bind(&authority_hash)
        .bind(basis)
        .bind(denominator_id)
        .bind(authority_set_id)
        .bind(request.legacy_allowed)
        .bind(assessment.control_decision.as_str())
        .bind(assessment.coverage_grade.as_str())
        .bind(assessment.divergence)
        .bind(expected)
        .bind(terminal)
        .bind(degraded)
        .bind(residual)
        .execute(&*self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reserve_local_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("reserve local postgres port")
            .local_addr()
            .expect("read reserved port")
            .port()
    }

    #[test]
    fn host_stage_blocked_is_terminal_only_with_evidence() {
        let updated_at = chrono::Utc::now();
        let mut outcome = HostStageOutcome {
            asset: "https://example.test".to_string(),
            technique: "WSTG-INFO".to_string(),
            outcome: "blocked".to_string(),
            source: Some("vuln_nuclei_general".to_string()),
            evidence_ids: Vec::new(),
            updated_at,
            seq: 1,
        };
        assert_eq!(host_stage_terminal_observation(Some(&outcome)), None);
        outcome.evidence_ids.push(42);
        assert_eq!(
            host_stage_terminal_observation(Some(&outcome)),
            Some("no_match")
        );
        outcome.outcome = "partial".to_string();
        assert_eq!(host_stage_terminal_observation(Some(&outcome)), None);
    }

    #[test]
    fn host_stage_recovery_uses_the_current_refresh_not_the_original_creation_time() {
        let stage_started_at = chrono::Utc::now();
        let outcome = HostStageOutcome {
            asset: "https://example.test:443".to_string(),
            technique: "GOLISH-ENUM-JS".to_string(),
            outcome: "found".to_string(),
            source: Some("browser_collect_js_api".to_string()),
            evidence_ids: vec![42],
            updated_at: stage_started_at + chrono::Duration::seconds(1),
            seq: 1,
        };

        assert!(host_stage_outcome_is_fresh(&outcome, stage_started_at));
        assert!(!host_stage_outcome_is_fresh(
            &outcome,
            outcome.updated_at + chrono::Duration::seconds(1),
        ));
    }

    #[test]
    fn eas_closeout_strengthens_empty_placeholders_from_exact_blocked_producer_evidence() {
        let organization_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        let created_at = chrono::Utc::now();
        let blocked = |technique: &str, evidence_id: i64| ExactHostStageEvidenceFact {
            evidence_asset: "61.130.180.110".to_string(),
            evidence_technique: technique.to_string(),
            evidence_outcome: "blocked".to_string(),
            evidence_id,
            evidence_organization_id: organization_id.to_string(),
            tool_name: Some("eas_discover_ports".to_string()),
            evidence_kind: Some("eas.port_scan_policy_blocked".to_string()),
            evidence_raw_output: Some(
                serde_json::json!({
                    "schema": "eas_port_scan_policy_blocked_v1",
                    "reason_code": "EAS_PORT_SCAN_ATTEMPTS_EXHAUSTED",
                    "scan_profile": "full",
                    "host_budget": 4,
                    "network_launched": false,
                    "target_id": target_id,
                })
                .to_string(),
            ),
            target_id,
            target_organization_id: Some(organization_id),
            target_type: "ip".to_string(),
            target_name: "61.130.180.110".to_string(),
            target_value: "61.130.180.110".to_string(),
            target_ports: serde_json::json!([]),
            created_at,
        };
        let evidence = vec![
            blocked(golish_db::repo::coverage_truth::TECH_EAS_LIVENESS, 402),
            blocked(golish_db::repo::coverage_truth::TECH_EAS_PORT, 403),
        ];
        let mut outcomes = vec![HostStageOutcome {
            asset: "61.130.180.110".to_string(),
            technique: golish_db::repo::coverage_truth::TECH_EAS_PORT.to_string(),
            outcome: "blocked".to_string(),
            source: Some("submit_stage_deliverable".to_string()),
            evidence_ids: Vec::new(),
            updated_at: created_at - chrono::Duration::seconds(1),
            seq: 1,
        }];

        strengthen_host_stage_outcomes_from_exact_evidence(
            "external_attack_surface",
            organization_id,
            &evidence,
            &mut outcomes,
        );

        for technique in [
            golish_db::repo::coverage_truth::TECH_EAS_LIVENESS,
            golish_db::repo::coverage_truth::TECH_EAS_PORT,
            golish_db::repo::coverage_truth::TECH_EAS_SERVICE_FP,
        ] {
            assert!(outcomes.iter().any(|outcome| {
                outcome.technique == technique
                    && outcome.outcome == "blocked"
                    && !outcome.evidence_ids.is_empty()
            }));
        }
        let mut foreign = evidence;
        foreign[0].target_organization_id = Some(Uuid::new_v4());
        let mut rejected = Vec::new();
        strengthen_host_stage_outcomes_from_exact_evidence(
            "external_attack_surface",
            organization_id,
            &foreign[..1],
            &mut rejected,
        );
        assert!(rejected.is_empty());
    }

    #[test]
    fn vuln_host_stage_resolves_canonical_origin_and_only_one_domain_alias() {
        let denominator_id = Uuid::new_v4();
        let item = |exact_asset: &str, target_type: &str| HostStageRootItem {
            denominator_id,
            project_path_at_freeze: "/tmp/tool-truth-origin".to_string(),
            input_key: format!("{target_type}:{exact_asset}"),
            target_id: Uuid::new_v4(),
            exact_asset: exact_asset.to_string(),
            target_type: target_type.to_string(),
            technique: "WSTG-INFO".to_string(),
            expected_capability: "vuln.nuclei_general".to_string(),
        };
        let items = vec![
            item("https://moresec.cn", "url"),
            item("moresec.cn", "domain"),
        ];
        let outcomes = vec![HostStageOutcome {
            asset: "https://moresec.cn:443".to_string(),
            technique: "WSTG-INFO".to_string(),
            outcome: "blocked".to_string(),
            source: Some("vuln_nuclei_general".to_string()),
            evidence_ids: vec![42],
            updated_at: chrono::Utc::now(),
            seq: 1,
        }];

        assert_eq!(
            host_stage_outcome_for_item("vuln_triage", &items[0], &items, &outcomes)
                .map(|outcome| outcome.asset.as_str()),
            Some("https://moresec.cn:443")
        );
        assert_eq!(
            host_stage_outcome_for_item("vuln_triage", &items[1], &items, &outcomes)
                .map(|outcome| outcome.asset.as_str()),
            Some("https://moresec.cn:443")
        );

        let ambiguous_items = vec![
            item("https://moresec.cn", "url"),
            item("http://moresec.cn", "url"),
            item("moresec.cn", "domain"),
        ];
        assert!(host_stage_outcome_for_item(
            "vuln_triage",
            &ambiguous_items[2],
            &ambiguous_items,
            &outcomes,
        )
        .is_none());
    }

    #[test]
    fn legacy_vuln_root_uses_exact_enumeration_surface_without_rescanning_excluded_targets() {
        let denominator_id = Uuid::new_v4();
        let item = |exact_asset: &str, target_type: &str| HostStageRootItem {
            denominator_id,
            project_path_at_freeze: "/tmp/tool-truth-vuln-legacy".to_string(),
            input_key: format!("{target_type}:{exact_asset}"),
            target_id: Uuid::new_v4(),
            exact_asset: exact_asset.to_string(),
            target_type: target_type.to_string(),
            technique: "WSTG-INFO".to_string(),
            expected_capability: "vuln.nuclei_general".to_string(),
        };
        let items = vec![
            item("moresec.cn", "domain"),
            item("moresec.com.cn", "domain"),
            item("61.130.180.110", "ip"),
        ];
        let producer = HostStageOutcome {
            asset: "https://moresec.cn:443".to_string(),
            technique: "WSTG-INFO".to_string(),
            outcome: "blocked".to_string(),
            source: Some("vuln_nuclei_general".to_string()),
            evidence_ids: vec![42],
            updated_at: chrono::Utc::now(),
            seq: 1,
        };
        let authority = VulnEnumerationSurfaceAuthority {
            origins: BTreeSet::from(["https://moresec.cn:443".to_string()]),
            evidence_ids: vec![5, 6],
            gate_passed_at: producer.updated_at - chrono::Duration::minutes(1),
        };

        let projected =
            legacy_vuln_root_surface_outcomes(&items, std::slice::from_ref(&producer), &authority)
                .expect("legacy root must reconcile against immutable Enumeration authority");

        assert_eq!(projected.len(), 3);
        assert!(projected.iter().any(|outcome| {
            outcome.asset == "moresec.cn"
                && outcome.outcome == "blocked"
                && outcome.evidence_ids == vec![42]
        }));
        for excluded in ["moresec.com.cn", "61.130.180.110"] {
            assert!(projected.iter().any(|outcome| {
                outcome.asset == excluded
                    && outcome.outcome == "not_applicable"
                    && outcome.source.as_deref()
                        == Some("enumeration_final_seal:excluded_from_vuln_surface")
                    && outcome.evidence_ids == vec![5, 6]
            }));
        }

        let exact_foreign = item("https://not-authorized.example:443", "url");
        assert!(legacy_vuln_root_surface_outcomes(
            std::slice::from_ref(&exact_foreign),
            &[],
            &authority,
        )
        .expect("exact-origin drift remains an unresolved gap")
        .is_empty());
    }

    #[test]
    fn public_denominator_request_cannot_omit_members_or_rebind_a_source() {
        let stage_execution_id = Uuid::new_v4();
        let source_id = Uuid::new_v4();
        let request = SealToolTruthDenominatorRequest {
            stable_seal_request_id: stable_denominator_seal_request(stage_execution_id, source_id),
            stage_execution_id,
            source: ToolTruthDenominatorSourceRef::StageAssetWave {
                stage_asset_wave_id: source_id,
            },
        };
        assert_eq!(
            request.stable_seal_request_id,
            stable_denominator_seal_request(stage_execution_id, source_id)
        );
        assert_ne!(
            request.stable_seal_request_id,
            stable_denominator_seal_request(stage_execution_id, Uuid::new_v4())
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn tool_truth_shadow_write_is_operation_and_org_scoped() {
        let data_dir = tempfile::tempdir().expect("temporary postgres directory");
        let mut db = golish_db::GolishDb::start(golish_db::DbConfig {
            pg_data_dir: data_dir.path().join("pgdata"),
            port: reserve_local_port(),
            database: format!("tool_truth_shadow_{}", Uuid::new_v4().simple()),
            ..golish_db::DbConfig::default()
        })
        .await
        .expect("start isolated migrated postgres");
        let pool = db.pool();
        let session_id = Uuid::new_v4();
        let operation_id = Uuid::new_v4();
        let project_scope_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let outside_organization_id = Uuid::new_v4();
        let scope_decision_id = Uuid::new_v4();
        let scope_snapshot_id = Uuid::new_v4();
        let project_path = format!("/tmp/tool-truth-shadow-{}", Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO sessions(id,title,status,project_path) VALUES($1,'shadow','running',$2)",
        )
        .bind(session_id)
        .bind(&project_path)
        .execute(pool)
        .await
        .expect("insert session");
        sqlx::query("INSERT INTO tasks(id,session_id,title,input,status) VALUES($1,$2,'shadow','fixture','running')")
            .bind(operation_id)
            .bind(session_id)
            .execute(pool)
            .await
            .expect("insert operation task");
        sqlx::query("INSERT INTO project_scopes(project_scope_id,canonical_project_path,path_sha256) VALUES($1,$2,$3)")
            .bind(project_scope_id)
            .bind(&project_path)
            .bind(format!("sha256:{}", "1".repeat(64)))
            .execute(pool)
            .await
            .expect("insert project scope");
        sqlx::query("INSERT INTO operation_state(operation_id,profile,current_stage,runtime_memory_contract,project_scope_id) VALUES($1,'assessment','enumeration','legacy_v1',$2)")
            .bind(operation_id)
            .bind(project_scope_id)
            .execute(pool)
            .await
            .expect("insert operation state");
        sqlx::query("ALTER TABLE operation_state DISABLE TRIGGER operation_state_tool_truth_contract_immutable")
            .execute(pool)
            .await
            .expect("enable isolated shadow fixture");
        sqlx::query(
            "UPDATE operation_state SET tool_truth_contract='shadow_v1' WHERE operation_id=$1",
        )
        .bind(operation_id)
        .execute(pool)
        .await
        .expect("freeze shadow contract in isolated fixture");
        sqlx::query("ALTER TABLE operation_state ENABLE TRIGGER operation_state_tool_truth_contract_immutable")
            .execute(pool)
            .await
            .expect("restore contract guard");
        sqlx::query("INSERT INTO organizations(id,project_path,name) VALUES($1,$3,'Scoped'),($2,$3,'Outside')")
            .bind(organization_id)
            .bind(outside_organization_id)
            .bind(&project_path)
            .execute(pool)
            .await
            .expect("insert organizations");
        sqlx::query("INSERT INTO stage_runs(id,operation_id,stage_kind,status) VALUES($1,$2,'enumeration','started')")
            .bind(stage_execution_id)
            .bind(operation_id)
            .execute(pool)
            .await
            .expect("insert active stage execution");
        sqlx::query(
            r#"INSERT INTO operation_scope_decisions(
                   id,operation_id,project_scope_id,stage_execution_id,
                   root_organization_id,mode,decision_rows,decision_hash
               ) VALUES($1,$2,$3,$4,$5,'cli_flags','[]',$6)"#,
        )
        .bind(scope_decision_id)
        .bind(operation_id)
        .bind(project_scope_id)
        .bind(stage_execution_id)
        .bind(organization_id)
        .bind(format!("sha256:{}", "2".repeat(64)))
        .execute(pool)
        .await
        .expect("insert scope decision");
        let mut tx = pool.begin().await.expect("begin scope freeze");
        sqlx::query(
            r#"INSERT INTO operation_org_scope_snapshots(
                   id,operation_id,project_scope_id,scope_decision_id,
                   project_path_at_freeze,root_organization_id,mode,scope_hash
               ) VALUES($1,$2,$3,$4,$5,$6,'cli_flags',$7)"#,
        )
        .bind(scope_snapshot_id)
        .bind(operation_id)
        .bind(project_scope_id)
        .bind(scope_decision_id)
        .bind(&project_path)
        .bind(organization_id)
        .bind(format!("sha256:{}", "3".repeat(64)))
        .execute(&mut *tx)
        .await
        .expect("insert scope snapshot");
        sqlx::query(
            r#"INSERT INTO operation_org_scope_units(
                   snapshot_id,organization_id,organization_name_at_freeze,
                   role,depth,ordinal,decision_row_id,approval_source
               ) VALUES($1,$2,'Scoped','root',0,0,'root','{}')"#,
        )
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .execute(&mut *tx)
        .await
        .expect("insert scoped organization");
        sqlx::query("UPDATE operation_org_scope_snapshots SET sealed_at=NOW() WHERE id=$1")
            .bind(scope_snapshot_id)
            .execute(&mut *tx)
            .await
            .expect("seal scope snapshot");
        tx.commit().await.expect("commit scope freeze");

        let provider = GolishDbRepoProvider::new(std::sync::Arc::new(pool.clone()));
        provider
            .tool_truth_record_shadow_assessment_impl(RecordToolTruthShadowAssessment {
                operation_id,
                organization_id,
                stage_kind: "enumeration".to_string(),
                stage_asset_wave_id: None,
                legacy_allowed: true,
            })
            .await
            .expect("record missing-denominator shadow assessment");
        let assessment: (String, String, bool) = sqlx::query_as(
            "SELECT control_decision,coverage_grade,divergence FROM tool_truth_gate_assessments WHERE operation_id=$1",
        )
        .bind(operation_id)
        .fetch_one(pool)
        .await
        .expect("read persisted shadow assessment");
        assert_eq!(
            assessment,
            ("hold".to_string(), "incomplete".to_string(), true)
        );

        let error = provider
            .tool_truth_record_shadow_assessment_impl(RecordToolTruthShadowAssessment {
                operation_id,
                organization_id: outside_organization_id,
                stage_kind: "enumeration".to_string(),
                stage_asset_wave_id: None,
                legacy_allowed: false,
            })
            .await
            .expect_err("outside organization cannot receive a scoped assessment");
        assert!(error.to_string().contains("no rows returned"));
        drop(provider);
        db.stop().await;
    }
}
