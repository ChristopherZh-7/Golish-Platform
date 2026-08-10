//! Fixture/dev composition for semantic Target Intel tools.
//!
//! Production bridge setup does not construct this executor. Tests/evals must
//! inject a fake collector, an evidence-first receipt store, and optionally a
//! Goal control plane for dynamic worker/reviewer primitives.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::harness::{
    IntelReviewBundle, IntelReviewBundleIdentity, IntelReviewReadCursor, IntelReviewSection,
    IntelReviewSectionKind,
};
use golish_agent_runtime::agentic_loop::McpToolExecutor;
use golish_pentest_domain::models::{AssetIntelPivot, AssetIntelPivotKind, IntelSearchIntent};
use golish_recon_app::asset_intel::{
    record_unsupported_semantic_query, run_fixture_semantic_query,
    semantic_fixture_capability_matrix, AssetIntelExecutionRequest, AssetIntelFixtureContext,
    AssetIntelHydrateConfig, CollectedIntelBatch, PassiveIntelSemanticSummary,
    ProjectionAuthorization, SemanticIntelReceiptStore, SemanticNativePivotPlanner,
    SemanticPlannedNativeQuery,
};
use golish_sub_agents::{
    adapt_target_intel_batch, render_neutral_reviewer_prompt, render_neutral_worker_prompt,
    IntelDynamicSpawnRequest, IntelGoalLeaderBinding, IntelReviewV1, IntelStampedWorkItem,
    STAGE_TEAM_REQUEST_INTEL_REVIEW,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[async_trait]
pub trait FakeSemanticIntelCollector: Send + Sync {
    fn is_fake(&self) -> bool;
    async fn collect(
        &self,
        query: &SemanticPlannedNativeQuery,
    ) -> Result<CollectedIntelBatch, String>;
}

#[async_trait]
pub trait IntelGoalFixtureControlPlane: Send + Sync {
    async fn record_semantic_summaries(
        &self,
        summaries: &[PassiveIntelSemanticSummary],
    ) -> Result<(), String>;
    async fn spawn_dynamic_workers(
        &self,
        request: IntelDynamicSpawnRequest,
    ) -> Result<Value, String>;
    async fn request_observe_only_review(&self, completion_claim: &str) -> Result<Value, String>;
}

#[async_trait]
pub trait FakeIntelGoalWorkerRunner: Send + Sync {
    fn is_fake(&self) -> bool;
    async fn execute_worker(
        &self,
        work_item: &IntelStampedWorkItem,
        host_prompt: &str,
    ) -> Result<Value, String>;
}

#[async_trait]
pub trait FakeIntelGoalReviewerRunner: Send + Sync {
    fn is_fake(&self) -> bool;
    async fn execute_reviewer(
        &self,
        bundle: &IntelReviewBundle,
        sections_in_host_order: &[IntelReviewSection],
        host_prompt: &str,
    ) -> Result<Value, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FixtureWorkerDisposition {
    Found,
    CheckedEmpty,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureWorkerOutput {
    business_disposition: FixtureWorkerDisposition,
    summary: String,
    fact_refs: Vec<Value>,
    evidence_ids: Vec<i64>,
    checked_empty_units: Vec<Value>,
    blocker_code: Option<String>,
}

impl FixtureWorkerOutput {
    fn parse(value: Value) -> Result<Self, String> {
        if serde_json::to_vec(&value).map_or(true, |bytes| bytes.len() > 256 * 1024) {
            return Err("INTEL_GOAL_WORKER_OUTPUT_BOUNDS_REJECTED".to_string());
        }
        let output: Self = serde_json::from_value(value)
            .map_err(|_| "INTEL_GOAL_WORKER_OUTPUT_CLOSED_SCHEMA_REJECTED".to_string())?;
        let evidence_ids = output.evidence_ids.iter().copied().collect::<BTreeSet<_>>();
        if output.summary.trim().is_empty()
            || output.summary.chars().count() > 4_000
            || output.summary.chars().any(char::is_control)
            || output.fact_refs.len() > 256
            || output.evidence_ids.len() > 256
            || output.checked_empty_units.len() > 256
            || output.evidence_ids.iter().any(|id| *id <= 0)
            || evidence_ids.len() != output.evidence_ids.len()
            || (output.business_disposition == FixtureWorkerDisposition::Found
                && output.fact_refs.is_empty()
                && output.evidence_ids.is_empty())
            || (output.business_disposition == FixtureWorkerDisposition::CheckedEmpty
                && output.checked_empty_units.is_empty()
                && output.evidence_ids.is_empty())
            || (output.business_disposition == FixtureWorkerDisposition::Blocked
                && output
                    .blocker_code
                    .as_deref()
                    .is_none_or(|code| code.trim().is_empty()))
            || (output.business_disposition != FixtureWorkerDisposition::Blocked
                && output.blocker_code.is_some())
        {
            return Err("INTEL_GOAL_WORKER_OUTPUT_NON_VACUITY_REJECTED".to_string());
        }
        Ok(output)
    }
}

#[derive(Debug, Default)]
struct InMemoryGoalState {
    completed: Vec<(IntelStampedWorkItem, FixtureWorkerOutput)>,
    accepted_keys: BTreeSet<String>,
    semantic_summaries: BTreeMap<String, PassiveIntelSemanticSummary>,
    review_round: u32,
}

/// Fake-only in-memory Goal control plane used by evals. It runs bounded
/// generic workers, freezes the exact four-section bundle, forces the reviewer
/// through the ordered cursor, and validates the closed review schema. It does
/// not grant production authority or mutate a StageTeam.
pub struct InMemoryIntelGoalFixtureControlPlane {
    leader: IntelGoalLeaderBinding,
    worker: Arc<dyn FakeIntelGoalWorkerRunner>,
    reviewer: Arc<dyn FakeIntelGoalReviewerRunner>,
    state: Mutex<InMemoryGoalState>,
}

impl InMemoryIntelGoalFixtureControlPlane {
    pub fn new(
        leader: IntelGoalLeaderBinding,
        worker: Arc<dyn FakeIntelGoalWorkerRunner>,
        reviewer: Arc<dyn FakeIntelGoalReviewerRunner>,
    ) -> Result<Self, &'static str> {
        if !leader.target_intel_fixture_bound || !worker.is_fake() || !reviewer.is_fake() {
            return Err("INTEL_GOAL_REAL_RUNNER_FORBIDDEN");
        }
        Ok(Self {
            leader,
            worker,
            reviewer,
            state: Mutex::new(InMemoryGoalState::default()),
        })
    }
}

#[async_trait]
impl IntelGoalFixtureControlPlane for InMemoryIntelGoalFixtureControlPlane {
    async fn record_semantic_summaries(
        &self,
        summaries: &[PassiveIntelSemanticSummary],
    ) -> Result<(), String> {
        let keyed = summaries
            .iter()
            .map(|summary| {
                summary
                    .query_receipts
                    .first()
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
                    .map(|key| (key, summary))
                    .ok_or_else(|| "INTEL_GOAL_SEMANTIC_RECEIPT_KEY_MISSING".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = self.state.lock();
        let new_keys = keyed
            .iter()
            .map(|(key, _)| key)
            .filter(|key| !state.semantic_summaries.contains_key(*key))
            .collect::<BTreeSet<_>>();
        if state.semantic_summaries.len() + new_keys.len() > 256 {
            return Err("INTEL_GOAL_SEMANTIC_SUMMARY_LIMIT_EXCEEDED".to_string());
        }
        if keyed.iter().any(|(key, summary)| {
            state
                .semantic_summaries
                .get(key)
                .is_some_and(|persisted| persisted != *summary && !summary.duplicate_terminal)
        }) {
            return Err("INTEL_GOAL_SEMANTIC_SUMMARY_REPLAY_MISMATCH".to_string());
        }
        for (key, summary) in keyed {
            state
                .semantic_summaries
                .entry(key)
                .or_insert_with(|| (*summary).clone());
        }
        Ok(())
    }

    async fn spawn_dynamic_workers(
        &self,
        request: IntelDynamicSpawnRequest,
    ) -> Result<Value, String> {
        let items =
            adapt_target_intel_batch(&self.leader, request).map_err(|error| error.to_string())?;
        let requested_count = items.len();
        let reserved = {
            let mut state = self.state.lock();
            let mut reserved = Vec::new();
            for item in items {
                if state.accepted_keys.insert(item.dedupe_key.clone()) {
                    reserved.push(item);
                }
            }
            if state.accepted_keys.len() > 128 {
                for item in &reserved {
                    state.accepted_keys.remove(&item.dedupe_key);
                }
                return Err("INTEL_GOAL_DYNAMIC_WORK_LIMIT_EXCEEDED".to_string());
            }
            reserved
        };
        let reserved_keys = reserved
            .iter()
            .map(|item| item.dedupe_key.clone())
            .collect::<Vec<_>>();
        let completed = async {
            let mut completed = Vec::with_capacity(reserved.len());
            for item in reserved {
                let prompt = render_neutral_worker_prompt(&item);
                let output = self.worker.execute_worker(&item, &prompt).await?;
                completed.push((item, FixtureWorkerOutput::parse(output)?));
            }
            Ok::<_, String>(completed)
        }
        .await;
        let completed = match completed {
            Ok(completed) => completed,
            Err(error) => {
                let mut state = self.state.lock();
                for key in reserved_keys {
                    state.accepted_keys.remove(&key);
                }
                return Err(error);
            }
        };
        let visible = completed
            .iter()
            .map(|(item, output)| {
                json!({
                    "display_name": item.display_name,
                    "prompt_sha256": item.prompt_sha256,
                    "subject_refs_sha256": item.subject_refs_sha256,
                    "dedupe_key": item.dedupe_key,
                    "business_disposition": output.business_disposition,
                    "fact_count": output.fact_refs.len(),
                    "evidence_count": output.evidence_ids.len(),
                    "checked_empty_count": output.checked_empty_units.len(),
                    "blocker_code": output.blocker_code,
                })
            })
            .collect::<Vec<_>>();
        let accepted_count = completed.len();
        let rejected_count = requested_count.saturating_sub(accepted_count);
        self.state.lock().completed.extend(completed);
        Ok(json!({
            "status": "completed",
            "accepted_count": accepted_count,
            "rejected_count": rejected_count,
            "workers": visible,
            "fixture_dev_only": true,
            "shadow_observe_only": true,
        }))
    }

    async fn request_observe_only_review(&self, completion_claim: &str) -> Result<Value, String> {
        let completion_claim = completion_claim.trim();
        if completion_claim.is_empty() || completion_claim.chars().count() > 12_000 {
            return Err("INTEL_GOAL_COMPLETION_CLAIM_INVALID".to_string());
        }
        let (completed, semantic_summaries, round) = {
            let mut state = self.state.lock();
            if state.accepted_keys.len() != state.completed.len() {
                return Err("INTEL_GOAL_WORKERS_STILL_ACTIVE".to_string());
            }
            if state.completed.is_empty() && state.semantic_summaries.is_empty() {
                return Err("INTEL_GOAL_REVIEW_NON_VACUITY_FAILED".to_string());
            }
            state.review_round = state
                .review_round
                .checked_add(1)
                .ok_or_else(|| "INTEL_GOAL_REVIEW_ROUND_OVERFLOW".to_string())?;
            (
                state.completed.clone(),
                state
                    .semantic_summaries
                    .values()
                    .cloned()
                    .collect::<Vec<_>>(),
                state.review_round,
            )
        };
        let durable_state = json!({
            "semantic_summaries": semantic_summaries.clone(),
            "worker_outputs": completed.iter().map(|(item, output)| json!({
                "dedupe_key": item.dedupe_key,
                "display_name": item.display_name,
                "subject_refs": item.subject_refs,
                "output": output,
            })).collect::<Vec<_>>()
        });
        let observable_actions = json!({
            "dynamic_worker_count": completed.len(),
            "semantic_receipt_count": semantic_summaries.len(),
            "artifact_refs": semantic_summaries.iter().flat_map(|summary| summary.artifact_refs.iter()).cloned().collect::<BTreeSet<_>>(),
            "prompt_hashes": completed.iter().map(|(item, _)| item.prompt_sha256.clone()).collect::<Vec<_>>(),
            "subject_set_hashes": completed.iter().map(|(item, _)| item.subject_refs_sha256.clone()).collect::<Vec<_>>(),
        });
        let frozen_contract = json!({
            "contract_version": "target_intel_goal.fixture.v1",
            "review_schema": "intel_review.v1",
            "runtime_mode": "observe_shadow",
            "completion_authority": "legacy_six_axis_v1",
            "fixture_dev_only": true,
            "shadow_observe_only": true,
        });
        let identity = fixture_review_identity(&self.leader, round);
        let bundle = IntelReviewBundle::freeze(
            identity,
            [
                durable_state,
                observable_actions,
                frozen_contract,
                json!({"completion_claim": completion_claim}),
            ],
        )
        .map_err(|error| error.to_string())?;
        let reviewer_worker_run_id = fixture_uuid(&self.leader, &format!("reviewer:{round}"));
        let mut cursor = IntelReviewReadCursor {
            review_id: bundle.identity.review_id,
            reviewer_worker_run_id,
            bundle_sha256: bundle.bundle_sha256.clone(),
            next_ordinal: 1,
        };
        let mut sections = Vec::with_capacity(4);
        for section in IntelReviewSectionKind::ORDER {
            sections.push(
                cursor
                    .read(&bundle, reviewer_worker_run_id, section)
                    .map_err(|error| error.to_string())?,
            );
        }
        if !cursor.completion_claim_read() {
            return Err("INTEL_GOAL_REVIEW_SECTIONS_INCOMPLETE".to_string());
        }
        let raw = self
            .reviewer
            .execute_reviewer(&bundle, &sections, render_neutral_reviewer_prompt())
            .await?;
        let review = IntelReviewV1::parse(raw).map_err(|error| error.to_string())?;
        Ok(json!({
            "status": "completed",
            "review_id": bundle.identity.review_id,
            "round": round,
            "bundle_sha256": bundle.bundle_sha256,
            "decision": review.verdict,
            "review": review,
            "all_four_sections_read": true,
            "fixture_dev_only": true,
            "shadow_observe_only": true,
        }))
    }
}

/// Compose the entire fake eval tool surface behind the same runtime custom
/// executor used by production MCP/custom tools. Every injectable runner and
/// collector must self-identify as fake.
pub fn build_fake_intel_goal_eval_executor(
    fixture_context: AssetIntelFixtureContext,
    projection_authorization: ProjectionAuthorization,
    leader: IntelGoalLeaderBinding,
    collector: Arc<dyn FakeSemanticIntelCollector>,
    receipt_store: Arc<dyn SemanticIntelReceiptStore>,
    worker: Arc<dyn FakeIntelGoalWorkerRunner>,
    reviewer: Arc<dyn FakeIntelGoalReviewerRunner>,
) -> Result<SemanticIntelFixtureExecutor, &'static str> {
    if fixture_context.operation_id != leader.operation_id
        || fixture_context.organization_id != leader.organization_id
        || fixture_context.operation_id.is_nil()
        || fixture_context.organization_id.is_nil()
        || fixture_context.session_id.is_nil()
    {
        return Err("INTEL_GOAL_EVAL_IDENTITY_MISMATCH");
    }
    let control_plane = Arc::new(InMemoryIntelGoalFixtureControlPlane::new(
        leader, worker, reviewer,
    )?);
    SemanticIntelFixtureExecutor::new(
        fixture_context,
        projection_authorization,
        collector,
        receipt_store,
    )
    .map(|executor| executor.with_control_plane(control_plane))
}

fn fixture_review_identity(
    leader: &IntelGoalLeaderBinding,
    round: u32,
) -> IntelReviewBundleIdentity {
    IntelReviewBundleIdentity {
        review_id: fixture_uuid(leader, &format!("review:{round}")),
        operation_id: leader.operation_id,
        stage_execution_id: fixture_uuid(leader, "stage_execution"),
        stage_run_unit_id: leader.stage_run_unit_id,
        organization_id: leader.organization_id,
        team_plan_id: fixture_uuid(leader, "team_plan"),
        controller_work_item_id: fixture_uuid(leader, "controller_work_item"),
        controller_worker_run_id: fixture_uuid(leader, "controller_worker_run"),
        controller_message_chain_id: fixture_uuid(leader, "controller_message_chain"),
        goal_epoch: leader.request_epoch,
        review_generation: i64::from(round),
        round,
        state_revision: i64::from(round),
    }
}

fn fixture_uuid(leader: &IntelGoalLeaderBinding, label: &str) -> Uuid {
    let digest = Sha256::digest(
        format!(
            "intel-goal-fixture:v1:{}:{}:{}:{}:{}",
            leader.operation_id,
            leader.organization_id,
            leader.stage_run_unit_id,
            leader.request_epoch,
            label
        )
        .as_bytes(),
    );
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

pub struct SemanticIntelFixtureExecutor {
    fixture_context: AssetIntelFixtureContext,
    projection_authorization: ProjectionAuthorization,
    legacy_config: AssetIntelHydrateConfig,
    collector: Arc<dyn FakeSemanticIntelCollector>,
    receipt_store: Arc<dyn SemanticIntelReceiptStore>,
    control_plane: Option<Arc<dyn IntelGoalFixtureControlPlane>>,
}

impl SemanticIntelFixtureExecutor {
    pub fn new(
        fixture_context: AssetIntelFixtureContext,
        projection_authorization: ProjectionAuthorization,
        collector: Arc<dyn FakeSemanticIntelCollector>,
        receipt_store: Arc<dyn SemanticIntelReceiptStore>,
    ) -> Result<Self, &'static str> {
        if !fixture_context.strict_passive
            || !fixture_context.fake_transport
            || !collector.is_fake()
        {
            return Err("INTEL_GOAL_REAL_TRANSPORT_FORBIDDEN");
        }
        Ok(Self {
            fixture_context,
            projection_authorization,
            legacy_config: AssetIntelHydrateConfig::default(),
            collector,
            receipt_store,
            control_plane: None,
        })
    }

    pub fn with_control_plane(
        mut self,
        control_plane: Arc<dyn IntelGoalFixtureControlPlane>,
    ) -> Self {
        self.control_plane = Some(control_plane);
        self
    }

    async fn record_goal_summaries(
        &self,
        summaries: &[PassiveIntelSemanticSummary],
    ) -> Result<(), String> {
        if let Some(control_plane) = &self.control_plane {
            control_plane.record_semantic_summaries(summaries).await?;
        }
        Ok(())
    }

    async fn execute_semantic_search(&self, args: &Value) -> (Value, bool) {
        let organization_id = match args
            .get("organization_id")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<Uuid>().ok())
        {
            Some(value) if value == self.fixture_context.organization_id => value,
            _ => {
                return (
                    json!({"error": "INTEL_GOAL_FOREIGN_OR_STALE_ORGANIZATION"}),
                    false,
                )
            }
        };
        let Some(pivot_input) = args.get("pivot").and_then(Value::as_object) else {
            return (json!({"error": "INTEL_PIVOT_INVALID"}), false);
        };
        let pivot_kind = match pivot_input
            .get("kind")
            .cloned()
            .and_then(|value| serde_json::from_value::<AssetIntelPivotKind>(value).ok())
        {
            Some(value) => value,
            None => return (json!({"error": "INTEL_PIVOT_KIND_INVALID"}), false),
        };
        let pivot = match pivot_input
            .get("value")
            .and_then(Value::as_str)
            .map(|value| AssetIntelPivot::parse(pivot_kind, value))
        {
            Some(Ok(value)) => value,
            Some(Err(error)) => return (json!({"error": error.to_string()}), false),
            None => return (json!({"error": "INTEL_PIVOT_VALUE_INVALID"}), false),
        };
        let intent = match args
            .get("intent")
            .cloned()
            .and_then(|value| serde_json::from_value::<IntelSearchIntent>(value).ok())
        {
            Some(value) => value,
            None => return (json!({"error": "INTEL_SEARCH_INTENT_INVALID"}), false),
        };
        let request = AssetIntelExecutionRequest {
            legacy_config: self.legacy_config.clone(),
            pivot: pivot.clone(),
            intent,
            projection_authorization: self.projection_authorization.clone(),
            fixture_context: AssetIntelFixtureContext {
                organization_id,
                ..self.fixture_context.clone()
            },
        };
        let plan = match SemanticNativePivotPlanner::plan(
            &pivot,
            intent,
            &semantic_fixture_capability_matrix(),
        ) {
            Ok(plan) => plan,
            Err(error) => {
                let summary = record_unsupported_semantic_query(
                    &request,
                    &error.capability,
                    &error.reason,
                    self.receipt_store.as_ref(),
                )
                .await;
                return match summary {
                    // Unsupported is a persisted terminal capability receipt,
                    // not an executor failure. Surface it as a successful tool
                    // observation so the Goal loop closes the pivot instead of
                    // retrying the same unavailable adapter forever.
                    Ok(summary) => match self
                        .record_goal_summaries(std::slice::from_ref(&summary))
                        .await
                    {
                        Ok(()) => (json!(summary), true),
                        Err(error) => (json!({"error": error.to_string()}), false),
                    },
                    Err(error) => (json!({"error": error.to_string()}), false),
                };
            }
        };
        let mut summaries = Vec::<PassiveIntelSemanticSummary>::new();
        for query in plan {
            let collected = match self.collector.collect(&query).await {
                Ok(collected) => collected,
                Err(reason) => {
                    return (
                        json!({
                            "error": "INTEL_FIXTURE_COLLECTION_FAILED",
                            "reason": reason,
                            "retryable": true
                        }),
                        false,
                    )
                }
            };
            match run_fixture_semantic_query(
                &request,
                &query,
                collected,
                self.receipt_store.as_ref(),
            )
            .await
            {
                Ok(summary) => summaries.push(summary),
                Err(error) => {
                    return (
                        json!({"error": error.to_string(), "retryable": true}),
                        false,
                    )
                }
            }
        }
        if let Err(error) = self.record_goal_summaries(&summaries).await {
            return (json!({"error": error}), false);
        }
        (
            json!({
                "fixture_dev_only": true,
                "shadow_observe_only": true,
                "pivot": pivot,
                "query_summaries": summaries
            }),
            true,
        )
    }
}

#[async_trait]
impl McpToolExecutor for SemanticIntelFixtureExecutor {
    async fn execute_tool(&self, tool_name: &str, args: &Value) -> Option<(Value, bool)> {
        match tool_name {
            "recon_search_intel" => Some(self.execute_semantic_search(args).await),
            golish_sub_agents::STAGE_TEAM_SPAWN_INTEL_SUBAGENTS => {
                let request = match serde_json::from_value::<IntelDynamicSpawnRequest>(args.clone())
                {
                    Ok(request) => request,
                    Err(_) => {
                        return Some((json!({"error": "INTEL_GOAL_CLOSED_SCHEMA_REJECTED"}), false))
                    }
                };
                let Some(control_plane) = self.control_plane.as_ref() else {
                    return Some((
                        json!({"error": "INTEL_GOAL_CONTROL_PLANE_UNAVAILABLE"}),
                        false,
                    ));
                };
                Some(match control_plane.spawn_dynamic_workers(request).await {
                    Ok(value) => (value, true),
                    Err(error) => (json!({"error": error}), false),
                })
            }
            STAGE_TEAM_REQUEST_INTEL_REVIEW => {
                let claim = args
                    .get("completion_claim")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty() && value.chars().count() <= 12_000);
                let (Some(claim), Some(control_plane)) = (claim, self.control_plane.as_ref())
                else {
                    return Some((
                        json!({"error": "INTEL_GOAL_REVIEW_CONTROL_UNAVAILABLE"}),
                        false,
                    ));
                };
                Some(
                    match control_plane.request_observe_only_review(claim).await {
                        Ok(value) => (value, true),
                        Err(error) => (json!({"error": error}), false),
                    },
                )
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NonFakeCollector;

    #[async_trait]
    impl FakeSemanticIntelCollector for NonFakeCollector {
        fn is_fake(&self) -> bool {
            false
        }

        async fn collect(
            &self,
            _query: &SemanticPlannedNativeQuery,
        ) -> Result<CollectedIntelBatch, String> {
            unreachable!()
        }
    }

    struct NoopStore;

    #[async_trait]
    impl SemanticIntelReceiptStore for NoopStore {
        async fn load_terminal_receipt(
            &self,
            _stable_query_key: &str,
        ) -> Result<Option<golish_recon_app::asset_intel::IntelPivotReceiptV1>, String> {
            Ok(None)
        }

        async fn save_redacted_artifact(
            &self,
            _artifact: &golish_recon_app::asset_intel::RedactedIntelArtifact,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn append_evidence(
            &self,
            _request: &AssetIntelExecutionRequest,
            _artifact: &golish_recon_app::asset_intel::RedactedIntelArtifact,
            _observations: &[golish_recon_app::asset_intel::CollectedIntelObservation],
        ) -> Result<String, String> {
            Ok("evidence:test".to_string())
        }

        async fn append_audit_receipt(
            &self,
            _receipt: &golish_recon_app::asset_intel::IntelPivotReceiptV1,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn semantic_fixture_executor_rejects_real_transport_before_dispatch() {
        let context = AssetIntelFixtureContext {
            operation_id: Uuid::from_u128(1),
            organization_id: Uuid::from_u128(2),
            session_id: Uuid::from_u128(3),
            strict_passive: true,
            fake_transport: true,
        };
        assert!(SemanticIntelFixtureExecutor::new(
            context,
            ProjectionAuthorization::default(),
            Arc::new(NonFakeCollector),
            Arc::new(NoopStore),
        )
        .is_err());
    }
}
