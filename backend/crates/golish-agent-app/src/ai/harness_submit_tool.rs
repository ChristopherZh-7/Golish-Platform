//! `submit_stage_deliverable` — deterministic StageDeliverable submission tool.
//!
//! Root cause this fixes (2026-06-02): the Primary orchestrator delegates the
//! deliverable to `sub_agent_reporter`, and both the reporter and the
//! orchestrator only ever *describe* the StageDeliverable in prose ("Generated
//! the JSON block with 5 claims…") — the parseable ```json block never
//! materialises anywhere. So the deterministic gate (which parses the
//! orchestrator's final text) always logs `gate skipped: no parseable
//! StageDeliverable`.
//!
//! A typed tool forces the model to emit *structured arguments* (it cannot
//! "describe" — it must fill `stage_id` / `claims` / `evidence_refs` fields; the
//! server assigns `stage_run_id`),
//! which the handler captures deterministically into the bridge side-channel
//! (`harness_last_deliverable`). The Task-mode executor then feeds it to the
//! gate at stage close. See `docs/design/2026-06-02-submit-stage-deliverable-tool.md`.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use uuid::Uuid;

use golish_agent_kit::db_traits::{
    CapturedStageSubmission, NewStageDeliverableSubmission, RuntimeMemoryError,
    RuntimeMemoryRepository, StageAssetWaveView, TechniqueOutcomeFact,
};
use golish_agent_kit::harness::org_gate::{
    apply_technique_outcome_rows, eas_service_not_applicable_from_port_outcomes,
    stage_accepts_outcome_projection, stage_accepts_source_query_completion,
    stage_asset_axis_cutoff, stage_gate_expected_techniques,
    trusted_vuln_surface_not_applicable_from_snapshot,
    validated_enumeration_axis_from_coverage_snapshot,
    validated_exact_web_origin_axis_from_coverage_snapshot, vuln_not_applicable_from_outcomes,
    TargetIntelOrganizationContext, STAGE_RUN_PASS_TOKEN_KIND,
};
use golish_agent_kit::harness::{
    completed_from_guarded_outcomes, load_embedded_stage_spec, validate_stage_gate_with_context,
    EvidenceFact, GateContext, GateContextBuilder, SourceQueryFact, StageClaim, StageDeliverable,
    StageKind,
};
use golish_agent_kit::runtime_memory::RuntimeMemoryWriteStrategy;
use golish_core::Tool;

const MAX_CANDIDATE_DECISION_GROUPS: usize = 100;

/// Narrow read-only seam over the evidence ledger so the submit tool can run the
/// fabricated-evidence cross-check without depending on the whole
/// `DbRepoProvider` surface (and so it is trivially mockable in tests). The app
/// wires it to `GolishDbRepoProvider::evidence_existing_ids`.
#[async_trait::async_trait]
pub trait EvidenceLedgerQuery: Send + Sync {
    /// Of `ids`, return the subset that actually exists in the evidence ledger.
    async fn existing_evidence_ids(&self, ids: &[i64]) -> Result<HashSet<i64>>;

    /// Recent **real** evidence ids for a chat session (newest first). Used to
    /// name the actually-citable ledger ids in a fabricated-ref `needs_fix`, so
    /// the model fills real ids instead of re-copying placeholders. Default empty
    /// so test doubles / no-DB paths need not implement it.
    async fn recent_evidence_ids(&self, session_id: &str, limit: i64) -> Result<Vec<i64>> {
        let _ = (session_id, limit);
        Ok(Vec::new())
    }

    /// Candidate V2 manifest projection for submit-time Gate validation. Missing
    /// implementations fail closed rather than returning an empty manifest.
    async fn candidate_manifest_work_item_keys(
        &self,
        operation_id: Uuid,
        stage_run_unit_id: Uuid,
        organization_id: Uuid,
    ) -> Result<Vec<String>> {
        let _ = (operation_id, stage_run_unit_id, organization_id);
        anyhow::bail!("ATTACK_V2_REPO_UNAVAILABLE")
    }

    /// Complete immutable Candidate manifest used to validate decision evidence
    /// before a worker is told that its submission was accepted.
    async fn candidate_manifest_for_unit(
        &self,
        operation_id: Uuid,
        stage_run_unit_id: Uuid,
        organization_id: Uuid,
    ) -> Result<golish_agent_kit::harness::attack_execution::CandidateManifestSnapshot> {
        let _ = (operation_id, stage_run_unit_id, organization_id);
        anyhow::bail!("ATTACK_V2_REPO_UNAVAILABLE")
    }

    /// Persisted-contract-aware Verification truth. `None` means the operation
    /// is legacy; `Some(empty)` means V2 truth was queried but no exact WaveUnit
    /// existed and must therefore block.
    async fn verification_truth_for_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<golish_agent_kit::harness::attack_execution::VerificationTruthSet>> {
        let _ = operation_id;
        anyhow::bail!("ATTACK_V2_VERIFICATION_TRUTH_UNAVAILABLE")
    }

    /// Current DB-authoritative Reporting revision truth for submit-time Gate
    /// preview. `None` is a real missing-revision state and must block.
    async fn reporting_truth_for_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<golish_agent_kit::harness::ReportingGateTruth>> {
        let _ = operation_id;
        anyhow::bail!("REPORTING_TRUTH_REPO_UNAVAILABLE")
    }

    /// Map real evidence ids to their ledger kind (`detail->>'kind'`). The submit
    /// tool uses this to reconcile legacy `min_invocations` checks from actual
    /// cited evidence instead of depending solely on the model-populated
    /// `required_checks_done` hint.
    async fn evidence_kinds_for(&self, ids: &[i64]) -> Result<HashMap<i64, String>> {
        let _ = ids;
        Ok(HashMap::new())
    }

    /// PR3 coverage projection · the session's evidence facts
    /// `(asset, technique, outcome)` derived from the audit-log columns. The
    /// submit-time gate preview feeds these into the coverage gate so an
    /// `authoritative_found` stage (e.g. target_intel) can credit a `found` cell
    /// from REAL ledger truth — instead of rejecting every found cell because the
    /// preview ran on an empty context. Default empty so test doubles / no-DB
    /// paths simply skip the projection (gate falls back to its prior behaviour).
    async fn evidence_facts(&self, session_id: &str) -> Vec<EvidenceFact> {
        let _ = session_id;
        Vec::new()
    }

    /// Strict EAS projection for submit preview. No default fallback to
    /// session-wide facts: providers that cannot prove producer org, current
    /// target ownership and stage freshness return no authoritative EAS facts.
    async fn eas_evidence_facts_for_session_org_fresh(
        &self,
        session_id: &str,
        organization_id: Uuid,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Vec<EvidenceFact> {
        let _ = (session_id, organization_id, since);
        Vec::new()
    }

    /// DB business-table truth facts (`organizations.asns/.certificates/.intel`
    /// → ASN/CT/OSINT, projected per in-scope asset) for `org_id`. Found-only
    /// (coverage_truth never infers checked_empty). This is the HALF of the gate
    /// context the submit preview previously missed: the session-keyed
    /// `evidence_facts` only carry command-path techniques (DNS/WHOIS/SUBDOMAIN
    /// from dig/whois/subfinder), never the org-keyed business-table ones
    /// (ASN/CT/OSINT — which have no CLI tool), so those cells were always
    /// "never attempted" and trapped per-org recon sub-agents in a resubmit loop.
    /// Default empty so test doubles / no-DB / no-org paths skip the projection.
    async fn db_truth_facts(&self, org_id: Option<Uuid>, assets: &[String]) -> Vec<EvidenceFact> {
        let _ = (org_id, assets);
        Vec::new()
    }

    async fn db_truth_facts_with_run_start(
        &self,
        org_id: Option<Uuid>,
        assets: &[String],
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<EvidenceFact> {
        let _ = run_start;
        self.db_truth_facts(org_id, assets).await
    }

    /// The authoritative in-scope asset values for `org_id` (org-isolated via
    /// `in_scope_values(None, org_id)`). Used as the coverage asset axis for the
    /// submit preview so it matches the org's real targets — NOT the whole-DB
    /// `in_scope_targets` set (which would drag in every target ever seeded
    /// across runs). Default empty → preview keeps the deliverable's
    /// self-reported asset set (prior behaviour).
    async fn in_scope_assets(&self, org_id: Option<Uuid>) -> Vec<String> {
        let _ = org_id;
        Vec::new()
    }

    async fn in_scope_assets_created_before(
        &self,
        org_id: Option<Uuid>,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Vec<String> {
        let _ = cutoff;
        self.in_scope_assets(org_id).await
    }

    async fn stage_asset_coverage(
        &self,
        organization_id: Uuid,
        stage: StageKind,
        session_id: Option<&str>,
        stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
        current_wave_target_ids: Option<Vec<Uuid>>,
        current_wave_asset_values: Option<Vec<String>>,
    ) -> Result<Option<Value>> {
        let _ = (
            organization_id,
            stage,
            session_id,
            stage_started_at,
            current_wave_target_ids,
            current_wave_asset_values,
        );
        Ok(None)
    }

    async fn stage_asset_coverage_for_operation(
        &self,
        operation_id: Option<Uuid>,
        organization_id: Uuid,
        stage: StageKind,
        session_id: Option<&str>,
        stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
        current_wave_target_ids: Option<Vec<Uuid>>,
        current_wave_asset_values: Option<Vec<String>>,
    ) -> Result<Option<Value>> {
        let _ = operation_id;
        self.stage_asset_coverage(
            organization_id,
            stage,
            session_id,
            stage_started_at,
            current_wave_target_ids,
            current_wave_asset_values,
        )
        .await
    }

    /// In-scope typed assets `(value, targets.type)` for `org_id`. Feeds the
    /// submit preview's host-aware `asset_types` so coverage_complete classifies
    /// assets by AUTHORITATIVE type (matching the stage-close gate) instead of
    /// value inference. Gated by `submit_preview_authoritative_context_enabled()`.
    /// Default empty ⇒ preview keeps value-inferred classes (prior behaviour).
    async fn in_scope_typed_assets(&self, org_id: Option<Uuid>) -> Vec<(String, String)> {
        let _ = org_id;
        Vec::new()
    }

    /// Distinct `targets.type` of the in-scope assets for `org_id`. Feeds the
    /// submit preview's dynamic `expected_techniques` (host-aware), matching the
    /// stage-close gate. Gated by `submit_preview_authoritative_context_enabled()`.
    /// Default empty ⇒ preview falls back to `spec.expected_techniques` (prior).
    async fn in_scope_target_types(&self, org_id: Option<Uuid>) -> Vec<String> {
        let _ = org_id;
        Vec::new()
    }

    /// EAS host-aware alias exclusion (设计 2026-06-30-eas-domain-port-delegation):
    /// in-scope asset values whose resolved IP is already an in-scope IP target,
    /// so the submit preview treats them as explanatory aliases of the IP row.
    /// Domains without a concrete IP remain liveness-only; PORT/SERVICE applies
    /// only to IP/CIDR via `technique_resolver`. Default empty ⇒ no exclusion.
    async fn eas_port_delegated_assets(&self, org_id: Option<Uuid>) -> Vec<String> {
        let _ = org_id;
        Vec::new()
    }

    /// Enumeration IP-web coverage (设计 2026-07-01 §5.3): in-scope IP/CIDR targets
    /// EAS/httpx proved are HTTP services (`targets.http_status` non-null). Feeds the
    /// submit preview's `web_capable_assets` so the preview matches the stage-close
    /// gate (org_gate) for enumeration IP-web roots — otherwise a web-capable IP
    /// passes the preview (dropped as not_applicable) but blocks at close. Default
    /// empty ⇒ preview keeps bare-IP exclusion (prior behaviour).
    async fn enumeration_web_capable_assets(&self, org_id: Option<Uuid>) -> Vec<String> {
        let _ = org_id;
        Vec::new()
    }

    /// EAS web-stack coverage denominator: all assets with a confirmed HTTP(S)
    /// surface in the current EAS freshness window.
    async fn eas_web_capable_assets(
        &self,
        org_id: Option<Uuid>,
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<String> {
        let _ = (org_id, run_start);
        Vec::new()
    }

    /// Current EAS exact-origin denominator. A real provider returns an error
    /// on query failure so submit preview can fail closed instead of accepting
    /// an empty denominator.
    async fn eas_required_web_origins(
        &self,
        organization_id: Uuid,
        since: chrono::DateTime<chrono::Utc>,
        current_wave_target_ids: Option<Vec<Uuid>>,
    ) -> Result<Vec<String>> {
        let _ = (organization_id, since, current_wave_target_ids);
        Ok(Vec::new())
    }

    /// Legacy DNS/53-only projection seam retained for repository compatibility.
    /// Exact-origin Enumeration excludes non-Web hosts from its denominator, and
    /// therefore must not turn raw host rows into origin-level not_applicable cells.
    async fn eas_service_not_applicable_assets(
        &self,
        org_id: Option<Uuid>,
        run_start: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<String> {
        let _ = (org_id, run_start);
        Vec::new()
    }

    /// #4/E3 (设计 2026-06-23-technique-outcomes-provenance): project
    /// `(asset, technique, outcome, evidence_id)` from the `technique_outcomes`
    /// table for the submit preview's dual-read (always on, no gray-switch).
    /// Default empty ⇒ preview doesn't read it (test doubles / read failure).
    async fn technique_outcome_facts(
        &self,
        org_id: Uuid,
        run_id: &str,
    ) -> Vec<TechniqueOutcomeFact> {
        let _ = (org_id, run_id);
        Vec::new()
    }

    async fn technique_outcome_facts_fresh(
        &self,
        org_id: Uuid,
        run_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<TechniqueOutcomeFact> {
        let _ = since;
        self.technique_outcome_facts(org_id, run_id).await
    }

    async fn technique_outcome_facts_fresh_with_evidence_session(
        &self,
        org_id: Uuid,
        outcome_run_id: &str,
        evidence_session_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<TechniqueOutcomeFact> {
        let _ = evidence_session_id;
        self.technique_outcome_facts_fresh(org_id, outcome_run_id, since)
            .await
    }

    /// #5 source/provider terminal rows for source coverage. Default empty so
    /// tests/no-DB paths keep prior behavior.
    async fn source_query_facts(&self, org_id: Uuid, run_id: &str) -> Vec<SourceQueryFact> {
        let _ = (org_id, run_id);
        Vec::new()
    }

    async fn operation_stage_started_at(
        &self,
        operation_id: Uuid,
    ) -> Option<(StageKind, chrono::DateTime<chrono::Utc>)> {
        let _ = operation_id;
        None
    }

    /// Trusted durable running-wave context for submit preview. The concrete DB
    /// bridge scopes this by `(operation, organization, stage)` and returns only
    /// the cutoff and original target values needed by the coverage projection.
    async fn stage_asset_wave_current_running(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        stage: StageKind,
    ) -> Result<Option<StageAssetWaveView>> {
        let _ = (operation_id, organization_id, stage);
        Ok(None)
    }

    async fn cleanup_closeout_gate(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
    ) -> Result<golish_agent_kit::db_traits::CleanupCloseoutGateSnapshot> {
        let _ = (operation_id, organization_id);
        anyhow::bail!("CLEANUP_CLOSEOUT_REPO_UNAVAILABLE")
    }
}

/// A background job that is still running or whose terminal completion has not
/// fully landed for the current session.
#[derive(Debug, Clone)]
pub struct RunningJobInfo {
    pub job_id: String,
    pub command: String,
    pub elapsed_ms: u64,
}

/// Narrow seam over the background-job manager so the submit tool can run the
/// closeout reconciliation barrier (Piece 3) without depending on
/// `golish-app-core` directly — and so it is trivially mockable in tests. The
/// app wires it to `golish_app_core::background_jobs::manager()`.
#[async_trait::async_trait]
pub trait BackgroundJobsQuery: Send + Sync {
    /// Wait for runtime-owned terminal reconciliation. Returns the jobs still
    /// outstanding when `max_wait` expires; empty means every side effect landed.
    async fn wait_for_session_reconciled(
        &self,
        session_id: &str,
        max_wait: std::time::Duration,
    ) -> Vec<RunningJobInfo>;
}

fn required_check_aliases_for_evidence_kind(kind: &str) -> &'static [&'static str] {
    match kind {
        "http_probe" => &["http_probe"],
        "dns_a" | "dns_aaaa" => &["dns_resolve"],
        "subdomain" | "subdomain_enum" | "subdomain_enum_passive" => &["subdomain_enum_passive"],
        _ => &[],
    }
}

fn collect_cited_evidence_ids(deliverable: &StageDeliverable) -> Vec<i64> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();

    let mut push = |id: i64| {
        if seen.insert(id) {
            ids.push(id);
        }
    };

    for evidence_id in &deliverable.evidence_refs {
        push(evidence_id.as_i64());
    }
    for claim in &deliverable.claims {
        for evidence_id in &claim.evidence_ids {
            push(evidence_id.as_i64());
        }
    }
    for finding in &deliverable.findings {
        for evidence_id in &finding.evidence_refs {
            push(evidence_id.as_i64());
        }
    }
    for cell in &deliverable.coverage {
        for evidence_id in &cell.evidence_refs {
            push(evidence_id.as_i64());
        }
    }

    ids
}

fn required_check_done_mentions(done: &str, alias: &str) -> bool {
    done.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .any(|token| token == alias)
}

fn backfill_required_checks_done_from_kinds(
    deliverable: &mut StageDeliverable,
    spec: &golish_agent_kit::harness::StageSpec,
    evidence_kinds: &HashMap<i64, String>,
) -> Vec<String> {
    let mut added = Vec::new();
    for evidence_id in collect_cited_evidence_ids(deliverable) {
        let Some(kind) = evidence_kinds.get(&evidence_id) else {
            continue;
        };
        for alias in required_check_aliases_for_evidence_kind(kind) {
            if !spec.min_invocations.contains_key(*alias) {
                continue;
            }
            let already = deliverable
                .required_checks_done
                .iter()
                .any(|done| required_check_done_mentions(done, alias));
            if !already {
                deliverable.required_checks_done.push((*alias).to_string());
                added.push((*alias).to_string());
            }
        }
    }
    added
}

fn canonicalize_model_submit_args(mut args: Value) -> Value {
    let Some(obj) = args.as_object_mut() else {
        return args;
    };

    for key in [
        "evidence_refs",
        "findings",
        "coverage",
        "skipped_checks",
        "required_checks_done",
    ] {
        if obj.get(key).is_some_and(Value::is_null) {
            obj.remove(key);
        }
    }

    if let Some(claims) = obj.get_mut("claims").and_then(Value::as_array_mut) {
        for claim in claims {
            let Some(claim_obj) = claim.as_object_mut() else {
                continue;
            };
            for key in ["evidence_ids", "technique"] {
                if claim_obj.get(key).is_some_and(Value::is_null) {
                    claim_obj.remove(key);
                }
            }
        }
    }

    // Scoping is a scope-decision stage, not a skipped-tool stage. Older prompts
    // taught models to encode "subsidiaries excluded" as `skipped_checks`, which
    // leaks the internal SkipReason enum and causes parse-time retry loops. The
    // canonical scoping deliverable carries that fact in the scope claim summary.
    if obj.get("stage_id").and_then(Value::as_str) == Some(StageKind::Scoping.as_str()) {
        obj.remove("skipped_checks");
    }

    args
}

/// RFC-8259 JSON with recursively lexicographic object keys and no insignificant
/// whitespace. JSONB does not retain input key order, so this exact string is
/// captured beside its SHA-256 and remains the deterministic Gate input.
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("JSON string serialization"),
        Value::Array(values) => {
            let values = values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{values}]")
        }
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let fields = keys
                .into_iter()
                .map(|key| {
                    let encoded_key =
                        serde_json::to_string(key).expect("JSON object-key serialization");
                    format!("{encoded_key}:{}", canonical_json(&object[key]))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{fields}}}")
        }
    }
}

fn sha256_hex(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Tool that captures a structured [`StageDeliverable`] into the bridge
/// side-channel so the deterministic gate can validate it, regardless of which
/// agent (orchestrator or `reporter`) produced it.
pub struct SubmitStageDeliverableTool {
    /// Active harness stage (set per-subtask by the Task-mode executor).
    active_stage: Arc<RwLock<Option<StageKind>>>,
    /// Sink the Task-mode executor reads at stage close + appends to content.
    last_deliverable: Arc<RwLock<Option<String>>>,
    /// V2 side-channel: exact durable submission identity + canonical Gate
    /// payload. The legacy string sink remains until the stage-close consumer is
    /// migrated, but V2 callers no longer have to infer identity from that JSON.
    captured_submission: Arc<RwLock<Option<CapturedStageSubmission>>>,
    /// Operation-scoped runtime-memory store. Absent only in explicit legacy
    /// fixtures/direct tests; production wiring always supplies the DB bridge.
    runtime_memory_repo: Option<Arc<dyn RuntimeMemoryRepository>>,
    /// P2 (validate-on-submit) · evidence-ledger handle. When present, the tool
    /// runs the same fabricated-evidence cross-check the stage-close gate uses,
    /// so a structurally-OK deliverable that cites non-existent ledger ids is
    /// returned as `needs_fix` *immediately* — instead of a misleading
    /// `accepted` that makes the agent jump ahead before the real gate blocks
    /// it. `None` = skip the check (e.g. tests / no DB), deferring to the gate.
    evidence_repo: Option<Arc<dyn EvidenceLedgerQuery>>,
    /// 乙 · chat-session string used to scope `recent_evidence_ids` so a
    /// fabricated-ref `needs_fix` can name the operation's REAL ids. `None` ⇒ no
    /// id hint (still rejects fabricated refs, just without the suggestion).
    session_id: Option<String>,
    /// Engagement root org id source (shared with the bridge's
    /// `harness_active_org_id`). Lets `gate_context` pull the org-keyed DB
    /// business-table truth (ASN/CT/OSINT) + authoritative asset axis for THIS
    /// run's org, mirroring the main-agent stage-close hook. `None` ⇒ no org
    /// binding ⇒ db-truth projection skipped (preview keeps prior behaviour).
    org_id_source: Option<Arc<RwLock<Option<Uuid>>>>,
    /// Active operation id source. Wave-aware stages use the operation state's
    /// `stage_started_at` as the current-wave asset cutoff so assets discovered
    /// during this stage do not move the submit-preview denominator.
    operation_id_source: Option<Arc<RwLock<Option<Uuid>>>>,
    /// Piece 3 (closeout reconciliation barrier) · background-job manager seam.
    /// When present (and a `session_id` is set), a submit that arrives while the
    /// session still has backgrounded scans running waits inside the same submit
    /// invocation until their terminal side effects are fully reconciled.
    /// `None` ⇒ barrier disabled (tests / no DI).
    bg_jobs: Option<Arc<dyn BackgroundJobsQuery>>,
    /// Total time the reconciliation barrier waits for runtime reconciliation.
    /// Production wires this from
    /// `GOLISH_SUBMIT_RECONCILE_WAIT_MS`.
    reconcile_wait_ms: u64,
}

fn findings_allowed_for_attack_contract(
    stage: StageKind,
    statically_allowed: bool,
    contract: golish_core::AttackExecutionContract,
) -> bool {
    stage != StageKind::VulnTriage
        && statically_allowed
        && !(contract.executes_v2_verifier()
            && matches!(
                stage,
                StageKind::VulnTriage | StageKind::AttackCandidate | StageKind::Verification
            ))
}

fn drop_disallowed_findings(deliverable: &mut StageDeliverable, findings_allowed: bool) -> usize {
    if findings_allowed || deliverable.findings.is_empty() {
        return 0;
    }
    let count = deliverable.findings.len();
    deliverable.findings.clear();
    count
}

/// Return the sole non-blank aggregate pass token. Other model-authored claims
/// are intentionally ignored later, but multiple competing token claims are
/// ambiguous and therefore cannot enter the coordinator closeout seam.
fn unique_stage_run_pass_token(deliverable: &StageDeliverable) -> Option<String> {
    let mut tokens = deliverable
        .claims
        .iter()
        .filter(|claim| claim.kind == STAGE_RUN_PASS_TOKEN_KIND)
        .map(|claim| claim.summary.trim())
        .filter(|token| !token.is_empty());
    let token = tokens.next()?.to_string();
    tokens.next().is_none().then_some(token)
}

impl SubmitStageDeliverableTool {
    pub fn new(
        active_stage: Arc<RwLock<Option<StageKind>>>,
        last_deliverable: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            active_stage,
            last_deliverable,
            captured_submission: Arc::new(RwLock::new(None)),
            runtime_memory_repo: None,
            evidence_repo: None,
            session_id: None,
            org_id_source: None,
            operation_id_source: None,
            bg_jobs: None,
            reconcile_wait_ms: 0,
        }
    }

    pub fn with_runtime_memory_repository(
        mut self,
        repo: Arc<dyn RuntimeMemoryRepository>,
    ) -> Self {
        self.runtime_memory_repo = Some(repo);
        self
    }

    /// Override the typed sink when a stage-close consumer owns the handle.
    pub fn with_captured_submission_sink(
        mut self,
        sink: Arc<RwLock<Option<CapturedStageSubmission>>>,
    ) -> Self {
        self.captured_submission = sink;
        self
    }

    pub fn captured_submission_handle(&self) -> Arc<RwLock<Option<CapturedStageSubmission>>> {
        Arc::clone(&self.captured_submission)
    }

    /// Expand the compact model-facing Candidate decision groups into the
    /// existing exact one-draft-per-work-item gate contract. The model may name
    /// exact frozen keys, one canonical manifest-kind prefix (for example,
    /// `surface_analysis:`), or exact Nuclei template ids. Selectors are
    /// expanded only against the trusted immutable manifest. Evidence ids are
    /// copied from that same manifest so repeated ids never consume model
    /// output and cannot drift across items.
    async fn expand_candidate_decision_groups(&self, args: &mut Value) -> Result<(), String> {
        let Some(object) = args.as_object_mut() else {
            return Ok(());
        };
        let Some(groups) = object.remove("candidate_decision_groups") else {
            return Ok(());
        };
        if object.get("stage_id").and_then(Value::as_str)
            != Some(StageKind::AttackCandidate.as_str())
        {
            return Err(
                "candidate_decision_groups is available only for attack_candidate".to_string(),
            );
        }
        if object
            .get("candidate_decisions")
            .and_then(Value::as_array)
            .is_some_and(|decisions| !decisions.is_empty())
        {
            return Err(
                "use candidate_decisions or candidate_decision_groups, never both".to_string(),
            );
        }
        let groups = groups
            .as_array()
            .ok_or_else(|| "candidate_decision_groups must be a non-empty array".to_string())?;
        if groups.is_empty() || groups.len() > MAX_CANDIDATE_DECISION_GROUPS {
            return Err(
                "candidate_decision_groups must contain 1..=100 bounded groups".to_string(),
            );
        }

        let repo = self.evidence_repo.as_ref().ok_or_else(|| {
            "Candidate decision-group expansion requires the trusted DB repository".to_string()
        })?;
        let context = golish_core::current_agent_tool_context().ok_or_else(|| {
            "Candidate decision-group expansion requires trusted tool context".to_string()
        })?;
        let operation_id = context.operation_id.ok_or_else(|| {
            "Candidate decision-group expansion requires operation identity".to_string()
        })?;
        let stage_run_unit_id = context.stage_run_unit_id.ok_or_else(|| {
            "Candidate decision-group expansion requires StageRunUnit identity".to_string()
        })?;
        let organization_id = context.organization_id.ok_or_else(|| {
            "Candidate decision-group expansion requires organization identity".to_string()
        })?;
        let manifest = repo
            .candidate_manifest_for_unit(operation_id, stage_run_unit_id, organization_id)
            .await
            .map_err(|error| format!("Candidate manifest load failed: {error}"))?;
        if manifest.operation_id != operation_id || manifest.organization_id != organization_id {
            return Err(
                "Candidate manifest returned a foreign operation or organization".to_string(),
            );
        }
        let manifest_by_key = manifest
            .work_items
            .iter()
            .map(|item| (item.work_item_key.as_str(), item))
            .collect::<HashMap<_, _>>();
        if manifest_by_key.len() != manifest.work_items.len() {
            return Err("Candidate manifest contains duplicate work-item keys".to_string());
        }

        const GROUP_FIELDS: &[&str] = &[
            "work_item_keys",
            "work_item_key_prefixes",
            "nuclei_template_ids",
            "decision",
            "hypothesis",
            "rationale",
            "no_candidate_reason_code",
        ];
        let mut expanded = Vec::new();
        let mut seen = HashSet::new();
        for group in groups {
            let group = group
                .as_object()
                .ok_or_else(|| "each Candidate decision group must be an object".to_string())?;
            if group
                .keys()
                .any(|key| !GROUP_FIELDS.contains(&key.as_str()))
            {
                return Err("Candidate decision groups contain an unsupported field".to_string());
            }
            let work_item_keys = group
                .get("work_item_keys")
                .and_then(Value::as_array)
                .filter(|keys| !keys.is_empty());
            let work_item_key_prefixes = group
                .get("work_item_key_prefixes")
                .and_then(Value::as_array)
                .filter(|prefixes| !prefixes.is_empty());
            let nuclei_template_ids = group
                .get("nuclei_template_ids")
                .and_then(Value::as_array)
                .filter(|template_ids| !template_ids.is_empty());
            if [
                work_item_keys.is_some(),
                work_item_key_prefixes.is_some(),
                nuclei_template_ids.is_some(),
            ]
            .into_iter()
            .filter(|selected| *selected)
            .count()
                != 1
            {
                return Err(
                    "each Candidate decision group needs exactly one selector: work_item_keys, work_item_key_prefixes, or nuclei_template_ids"
                        .to_string(),
                );
            }
            let decision = group
                .get("decision")
                .and_then(Value::as_str)
                .filter(|value| matches!(*value, "candidate" | "no_candidate"))
                .ok_or_else(|| {
                    "each Candidate decision group needs candidate/no_candidate".to_string()
                })?;
            let rationale = group
                .get("rationale")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    "each Candidate decision group needs a non-empty rationale".to_string()
                })?;
            if decision == "candidate"
                && nuclei_template_ids.is_some_and(|template_ids| template_ids.len() != 1)
            {
                return Err(
                    "a candidate Nuclei decision group must select exactly one template id so its hypothesis remains template-specific; metadata no_candidate groups may select multiple templates"
                        .to_string(),
                );
            }
            let selected_keys = if let Some(work_item_keys) = work_item_keys {
                work_item_keys
                    .iter()
                    .map(|key| {
                        key.as_str()
                            .filter(|value| !value.trim().is_empty())
                            .ok_or_else(|| {
                                "Candidate decision-group keys must be non-empty strings"
                                    .to_string()
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else if let Some(prefixes) = work_item_key_prefixes {
                if prefixes.len() > 3 {
                    return Err(
                        "Candidate decision groups accept at most three manifest-kind prefixes"
                            .to_string(),
                    );
                }
                let manifest_kind_prefixes = manifest
                    .work_items
                    .iter()
                    .filter_map(|item| {
                        item.work_item_key
                            .split_once(':')
                            .map(|(kind, _)| format!("{kind}:"))
                    })
                    .collect::<HashSet<_>>();
                let mut requested_prefixes = HashSet::new();
                for prefix in prefixes {
                    let prefix = prefix
                        .as_str()
                        .filter(|value| !value.trim().is_empty())
                        .ok_or_else(|| {
                            "Candidate decision-group prefixes must be non-empty strings"
                                .to_string()
                        })?;
                    if !manifest_kind_prefixes.contains(prefix) {
                        return Err(format!(
                            "Candidate decision-group prefix {prefix} is not a canonical manifest-kind prefix"
                        ));
                    }
                    if !requested_prefixes.insert(prefix) {
                        return Err(format!(
                            "Candidate decision-group prefix {prefix} is duplicated"
                        ));
                    }
                }
                let selected = manifest
                    .work_items
                    .iter()
                    .filter(|item| {
                        requested_prefixes
                            .iter()
                            .any(|prefix| item.work_item_key.starts_with(*prefix))
                    })
                    .map(|item| item.work_item_key.as_str())
                    .collect::<Vec<_>>();
                if selected.is_empty() {
                    return Err(
                        "Candidate decision-group prefixes selected no frozen work items"
                            .to_string(),
                    );
                }
                selected
            } else {
                let template_ids =
                    nuclei_template_ids.expect("exactly one selector was checked above");
                if template_ids.len() > 16 {
                    return Err(
                        "Candidate decision groups accept at most 16 Nuclei template ids"
                            .to_string(),
                    );
                }
                let mut requested_template_ids = HashSet::new();
                for template_id in template_ids {
                    let template_id = template_id
                        .as_str()
                        .filter(|value| {
                            !value.trim().is_empty()
                                && value.len() <= 128
                                && value.bytes().all(|byte| {
                                    byte.is_ascii_alphanumeric()
                                        || matches!(byte, b'-' | b'_' | b'.' | b'/')
                                })
                        })
                        .ok_or_else(|| {
                            "Candidate Nuclei template selectors must be bounded safe strings"
                                .to_string()
                        })?;
                    if !requested_template_ids.insert(template_id) {
                        return Err(format!(
                            "Candidate Nuclei template selector {template_id} is duplicated"
                        ));
                    }
                }
                let available_template_ids = manifest
                    .work_items
                    .iter()
                    .filter(|item| item.observation_kind == "nuclei_match_v1")
                    .filter_map(|item| item.observation.get("template_id").and_then(Value::as_str))
                    .collect::<HashSet<_>>();
                if let Some(unknown) = requested_template_ids
                    .iter()
                    .find(|template_id| !available_template_ids.contains(**template_id))
                {
                    return Err(format!(
                        "Candidate Nuclei template selector {unknown} is not present in the frozen manifest"
                    ));
                }
                let selected = manifest
                    .work_items
                    .iter()
                    .filter(|item| {
                        item.observation_kind == "nuclei_match_v1"
                            && item
                                .observation
                                .get("template_id")
                                .and_then(Value::as_str)
                                .is_some_and(|template_id| {
                                    requested_template_ids.contains(template_id)
                                })
                    })
                    .map(|item| item.work_item_key.as_str())
                    .collect::<Vec<_>>();
                if selected.is_empty() {
                    return Err(
                        "Candidate Nuclei template selectors selected no frozen work items"
                            .to_string(),
                    );
                }
                selected
            };
            for key in selected_keys {
                if !seen.insert(key.to_string()) {
                    return Err(format!(
                        "Candidate work item {key} appears in more than one decision group"
                    ));
                }
                let item = manifest_by_key.get(key).ok_or_else(|| {
                    format!("Candidate decision group named unknown work item {key}")
                })?;
                if item.evidence_ids.is_empty() {
                    return Err(format!(
                        "Candidate work item {key} has no frozen decision evidence"
                    ));
                }
                let mut draft = serde_json::Map::new();
                draft.insert("work_item_key".to_string(), json!(key));
                draft.insert("decision".to_string(), json!(decision));
                draft.insert("rationale".to_string(), json!(rationale));
                draft.insert("evidence_refs".to_string(), json!(item.evidence_ids));
                for optional in ["hypothesis", "no_candidate_reason_code"] {
                    if let Some(value) = group.get(optional).filter(|value| !value.is_null()) {
                        draft.insert(optional.to_string(), value.clone());
                    }
                }
                expanded.push(Value::Object(draft));
            }
        }
        if expanded.len() > MAX_CANDIDATE_DECISION_GROUPS {
            return Err("Candidate decision groups expand beyond 100 work items".to_string());
        }
        object.insert("candidate_decisions".to_string(), Value::Array(expanded));
        Ok(())
    }

    async fn attack_execution_contract(&self) -> Result<golish_core::AttackExecutionContract> {
        let Some(repo) = self.runtime_memory_repo.as_ref() else {
            // Explicit no-repository fixtures and direct legacy use retain the
            // historical deliverable contract. Production always injects the
            // repository and must resolve the persisted operation contract.
            return Ok(golish_core::AttackExecutionContract::Legacy);
        };
        let sourced_operation_id = match self.operation_id_source.as_ref() {
            Some(source) => *source.read().await,
            None => None,
        };
        let context_operation_id =
            golish_core::current_agent_tool_context().and_then(|context| context.operation_id);
        let operation_id = sourced_operation_id
            .or(context_operation_id)
            .ok_or_else(|| {
                anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                    code: "missing_operation_id",
                })
            })?;
        if sourced_operation_id.is_some_and(|trusted| trusted != operation_id)
            || context_operation_id.is_some_and(|trusted| trusted != operation_id)
        {
            return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                code: "submission_operation_identity_mismatch",
            }));
        }
        repo.attack_execution_contract_for_operation(operation_id)
            .await
            .map_err(anyhow::Error::new)
    }

    async fn effective_findings_allowed(
        &self,
        stage: StageKind,
        statically_allowed: bool,
    ) -> Result<bool> {
        if !statically_allowed {
            return Ok(false);
        }
        if !matches!(
            stage,
            StageKind::VulnTriage | StageKind::AttackCandidate | StageKind::Verification
        ) {
            return Ok(statically_allowed);
        }
        let contract = self.attack_execution_contract().await?;
        Ok(findings_allowed_for_attack_contract(
            stage,
            statically_allowed,
            contract,
        ))
    }

    async fn persist_trusted_submission(
        &self,
        active_stage: StageKind,
        deliverable: &mut StageDeliverable,
        coordinator_pass_token: Option<String>,
    ) -> Result<Option<CapturedStageSubmission>> {
        let Some(repo) = self.runtime_memory_repo.as_ref() else {
            // Explicit legacy fixture/direct-test seam. Production registers the
            // runtime repository and cannot silently enter this branch.
            deliverable.stage_run_id = Uuid::new_v4();
            return Ok(None);
        };

        let context = golish_core::current_agent_tool_context();
        let sourced_operation_id = match self.operation_id_source.as_ref() {
            Some(source) => *source.read().await,
            None => None,
        };
        let context_operation_id = context.as_ref().and_then(|context| context.operation_id);
        let operation_id = sourced_operation_id
            .or(context_operation_id)
            .ok_or_else(|| {
                anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                    code: "missing_operation_id",
                })
            })?;
        if sourced_operation_id.is_some_and(|trusted| trusted != operation_id)
            || context_operation_id.is_some_and(|trusted| trusted != operation_id)
        {
            return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                code: "submission_operation_identity_mismatch",
            }));
        }

        let contract = repo
            .runtime_memory_contract_for_operation(operation_id)
            .await
            .map_err(anyhow::Error::new)?;
        if contract.policy().write == RuntimeMemoryWriteStrategy::LegacyOnly {
            deliverable.stage_run_id = Uuid::new_v4();
            return Ok(None);
        }

        let context = context.ok_or_else(|| {
            anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                code: "missing_trusted_tool_context",
            })
        })?;
        if context.operation_id != Some(operation_id) {
            return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                code: "submission_operation_identity_mismatch",
            }));
        }
        if context.tool_name != self.name() {
            return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                code: "submission_tool_name_mismatch",
            }));
        }
        let stage_execution_id = context.stage_execution_id.ok_or_else(|| {
            anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                code: "missing_stage_execution_id",
            })
        })?;
        let tool_call_record_id = context.tool_call_record_id.ok_or_else(|| {
            anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                code: "missing_tool_call_record_id",
            })
        })?;

        // A specialist stage has two deliberately different closeout records:
        // each org worker persists its immutable V2 StageDeliverable against a
        // unit+lease, while the top-level coordinator only echoes the
        // deterministic stage_run pass token. The latter is re-derived from
        // org_stage_completions by the final gate and is not a second business
        // deliverable, so it must not be forced into the per-unit submission
        // table. Keep the bypass narrow to a trusted main-agent call with no
        // unit/worker identity; a worker can never use it to evade its fence.
        let is_trusted_coordinator_closeout = coordinator_pass_token.is_some()
            && matches!(context.source, golish_core::events::ToolSource::Main)
            && context.stage_run_unit_id.is_none()
            && context.worker_lease.is_none()
            && context.candidate_attempt.is_none();
        if is_trusted_coordinator_closeout {
            let pass_token = coordinator_pass_token.expect("checked coordinator pass token");
            // Aggregate closeout carries no model-authored business facts. The
            // org workers already persisted those under their exact units; the
            // final gate needs only the server-normalized token to recompute.
            deliverable.stage_id = active_stage.as_str().to_string();
            deliverable.stage_run_id = stage_execution_id;
            deliverable.claims = vec![StageClaim {
                kind: STAGE_RUN_PASS_TOKEN_KIND.to_string(),
                subject: active_stage.as_str().to_string(),
                summary: pass_token,
                evidence_ids: Vec::new(),
                technique: None,
            }];
            deliverable.evidence_refs.clear();
            deliverable.skipped_checks.clear();
            deliverable.findings.clear();
            deliverable.required_checks_done.clear();
            deliverable.coverage.clear();
            deliverable.candidates.clear();
            deliverable.candidate_decisions.clear();
            // The bridge prefers a typed durable capture over the legacy JSON
            // capture. Remove any same-stage residue so the coordinator token
            // written below is the payload seen by the aggregate closeout gate.
            *self.captured_submission.write().await = None;
            return Ok(None);
        }

        let stage_run_unit_id = context.stage_run_unit_id;
        let mut organization_id = context.organization_id;
        let worker = context.worker_lease.as_ref();
        if active_stage == StageKind::Scoping {
            if stage_run_unit_id.is_none() {
                // A pre-bound engagement org is an authorization scope, not a
                // durable unit owner. Preliminary Scoping submissions remain
                // execution-only until scope freeze binds the root unit+org.
                organization_id = None;
            } else if organization_id.is_none() {
                return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                    code: "scoping_stage_unit_organization_shape_mismatch",
                }));
            }
            if worker.is_some() {
                return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                    code: "scoping_submission_must_not_bind_worker",
                }));
            }
        } else {
            if stage_run_unit_id.is_none() {
                return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                    code: "missing_stage_run_unit",
                }));
            }
            if organization_id.is_none() {
                return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                    code: "missing_stage_organization",
                }));
            }
            if worker.is_none() {
                return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                    code: "missing_worker_run",
                }));
            }
        }
        if let Some(worker) = worker {
            if Some(worker.stage_run_unit_id) != stage_run_unit_id {
                return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                    code: "worker_stage_run_unit_mismatch",
                }));
            }
        }

        deliverable.stage_run_id = stage_execution_id;
        let payload = serde_json::to_value(&*deliverable)?;
        let canonical_deliverable_json = canonical_json(&payload);
        let payload_sha256 = sha256_hex(&canonical_deliverable_json);
        let persisted = repo
            .insert_stage_deliverable_submission(NewStageDeliverableSubmission {
                operation_id,
                stage_execution_id,
                stage_run_unit_id,
                worker_run_id: worker.map(|worker| worker.worker_run_id),
                organization_id,
                tool_call_record_id,
                tool_request_id: context.request_id,
                stage_kind: active_stage.as_str().to_string(),
                attempt_epoch: worker.map(|worker| worker.attempt_epoch),
                lease_token: worker.map(|worker| worker.lease_token),
                canonical_deliverable_json: canonical_deliverable_json.clone(),
                payload_sha256: payload_sha256.clone(),
            })
            .await
            .map_err(anyhow::Error::new)?;
        if persisted.operation_id != operation_id
            || persisted.stage_execution_id != stage_execution_id
            || persisted.stage_run_unit_id != stage_run_unit_id
            || persisted.tool_call_record_id != tool_call_record_id
            || persisted.payload_sha256 != payload_sha256
        {
            return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                code: "persisted_submission_identity_mismatch",
            }));
        }
        let captured = CapturedStageSubmission {
            deliverable_submission_id: persisted.deliverable_submission_id,
            operation_id,
            stage_execution_id,
            stage_run_unit_id,
            canonical_deliverable_json: canonical_deliverable_json.clone(),
            payload_sha256,
        };
        *self.captured_submission.write().await = Some(captured.clone());
        // V2 callers consume the exact durable submission id from their own
        // tool result. Do not publish it through the shared legacy
        // last-deliverable slot, which can leak residue across serial workers.
        Ok(Some(captured))
    }

    async fn cleanup_gate_block_reason(&self) -> Option<String> {
        let Some(repo) = self.evidence_repo.as_ref() else {
            return Some("cleanup authoritative repository is unavailable".to_string());
        };
        let context = golish_core::current_agent_tool_context();
        let sourced_operation = match self.operation_id_source.as_ref() {
            Some(source) => *source.read().await,
            None => None,
        };
        let context_operation = context.as_ref().and_then(|value| value.operation_id);
        if sourced_operation.is_some()
            && context_operation.is_some()
            && sourced_operation != context_operation
        {
            return Some("cleanup operation identity sources disagree".to_string());
        }
        let sourced_org = match self.org_id_source.as_ref() {
            Some(source) => *source.read().await,
            None => None,
        };
        let context_org = context.as_ref().and_then(|value| value.organization_id);
        if sourced_org.is_some() && context_org.is_some() && sourced_org != context_org {
            return Some("cleanup organization identity sources disagree".to_string());
        }
        let Some(operation_id) = context_operation.or(sourced_operation) else {
            return Some("cleanup gate has no exact operation identity".to_string());
        };
        let Some(organization_id) = context_org.or(sourced_org) else {
            return Some("cleanup gate has no exact organization identity".to_string());
        };
        match repo
            .cleanup_closeout_gate(operation_id, organization_id)
            .await
        {
            Ok(snapshot) if snapshot.allows_closeout() => None,
            Ok(snapshot) => Some(format!(
                "cleanup DB truth blocks closeout: missing_obligations={}, nonterminal_obligations={}, undisclosed_residuals={}, invalid_terminal_truth={}",
                snapshot.missing_obligation_count,
                snapshot.nonterminal_obligation_count,
                snapshot.undisclosed_residual_count,
                snapshot.invalid_terminal_truth_count,
            )),
            Err(error) => Some(format!(
                "cleanup authoritative closeout query failed: {error}"
            )),
        }
    }

    fn attach_submission_identity(
        mut response: Value,
        captured: Option<&CapturedStageSubmission>,
    ) -> Value {
        let Some(captured) = captured else {
            return response;
        };
        let Some(object) = response.as_object_mut() else {
            return response;
        };
        object.insert(
            "deliverable_submission_id".to_string(),
            json!(captured.deliverable_submission_id),
        );
        object.insert(
            "stage_execution_id".to_string(),
            json!(captured.stage_execution_id),
        );
        object.insert(
            "stage_run_unit_id".to_string(),
            json!(captured.stage_run_unit_id),
        );
        response
    }

    /// Attach an evidence-ledger handle to enable validate-on-submit (P2).
    pub fn with_evidence_repo(mut self, repo: Arc<dyn EvidenceLedgerQuery>) -> Self {
        self.evidence_repo = Some(repo);
        self
    }

    /// Scope the real-id suggestion (乙) to this chat session.
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Share the bridge's active engagement-org id so the submit preview can
    /// project the org-keyed DB business-table truth (ASN/CT/OSINT) + the
    /// authoritative in-scope asset axis into the gate context.
    pub fn with_org_id_source(mut self, src: Arc<RwLock<Option<Uuid>>>) -> Self {
        self.org_id_source = Some(src);
        self
    }

    /// Share the bridge's active operation id so wave-aware stage submit previews
    /// can freeze the coverage denominator to assets present when the stage
    /// started.
    pub fn with_operation_id_source(mut self, src: Arc<RwLock<Option<Uuid>>>) -> Self {
        self.operation_id_source = Some(src);
        self
    }

    /// Enable the closeout reconciliation barrier (Piece 3): a submit that arrives
    /// while this session still has backgrounded scans running defers before the
    /// gate preview. Needs a `session_id` to attribute jobs; without one the
    /// barrier is inert.
    pub fn with_background_jobs(mut self, bg: Arc<dyn BackgroundJobsQuery>) -> Self {
        self.bg_jobs = Some(bg);
        self
    }

    /// Configure the reconciliation barrier's wait budget. The second argument
    /// remains for source compatibility with older callers; waiting is now
    /// event-driven and does not poll.
    /// Production reads `GOLISH_SUBMIT_RECONCILE_WAIT_MS`; tests pass small values
    /// to exercise the timeout branch without real delay.
    pub fn with_reconcile_timeouts(mut self, wait_ms: u64, _poll_ms: u64) -> Self {
        self.reconcile_wait_ms = wait_ms;
        self
    }

    /// Closeout reconciliation barrier (Piece 3). Returns `Some(needs_fix json)`
    /// only when the runtime-owned event wait exceeds `reconcile_wait_ms`.
    /// Healthy completions continue in this same invocation without asking the
    /// model to call `check_job` / `wait_for_background_jobs` and resubmit.
    async fn reconcile_background_jobs(&self) -> Option<Value> {
        let (bg, sid) = (self.bg_jobs.as_ref()?, self.session_id.as_deref()?);
        let running = bg
            .wait_for_session_reconciled(
                sid,
                std::time::Duration::from_millis(self.reconcile_wait_ms),
            )
            .await;
        if running.is_empty() {
            return None;
        }
        tracing::info!(
            target: "harness::submit_tool",
            session_id = %sid,
            still_running = running.len(),
            "submit deferred: background reconciliation deadline expired"
        );
        let jobs: Vec<Value> = running
            .iter()
            .map(|j| {
                json!({
                    "job_id": j.job_id,
                    "command": j.command,
                    "elapsed_ms": j.elapsed_ms,
                })
            })
            .collect();
        Some(json!({
            "status": "needs_fix",
            "reasons": [format!(
                "System reconciliation deadline expired with {} background job(s) still \
                 running or awaiting result landing. Do NOT re-run the same command. The \
                 runtime normally resumes this submit automatically; this is an exceptional \
                 recovery path. Inspect a listed job at most once with check_job, and use \
                 kill_job only if it is genuinely stuck, then submit again after its terminal \
                 notification.",
                running.len()
            )],
            "running_background_jobs": jobs,
            "note": "runtime reconciliation timed out; use the background-job detail and one-shot control tools only for recovery."
        }))
    }

    /// The operation's real evidence ids (newest first), exposed only as debug
    /// context when the model cited fabricated refs. Empty when no repo / no
    /// session / infra error.
    async fn available_real_ids(&self) -> Vec<i64> {
        let (Some(repo), Some(sid)) = (self.evidence_repo.as_ref(), self.session_id.as_deref())
        else {
            return Vec::new();
        };
        repo.recent_evidence_ids(sid, 25).await.unwrap_or_default()
    }

    /// Cross-check any model-supplied evidence ids against the real ledger.
    /// Returns the cited ids that do NOT exist (fabricated), in cited order.
    /// An infra error is treated as "can't prove fabrication" → empty (the
    /// authoritative stage-close gate still runs), mirroring the orchestrator's
    /// fail-open behaviour so DB blips never wedge a legitimate stage.
    async fn fabricated_refs(&self, deliverable: &StageDeliverable) -> Vec<i64> {
        let Some(repo) = self.evidence_repo.as_ref() else {
            return Vec::new();
        };
        let cited = collect_cited_evidence_ids(deliverable);
        if cited.is_empty() {
            return Vec::new();
        }
        match repo.existing_evidence_ids(&cited).await {
            Ok(existing) => cited
                .into_iter()
                .filter(|id| !existing.contains(id))
                .collect(),
            Err(e) => {
                tracing::warn!(
                    target: "harness::submit_tool",
                    error = %e,
                    "evidence existence check failed; deferring to stage-close gate"
                );
                Vec::new()
            }
        }
    }

    async fn backfill_required_checks_done_from_evidence(
        &self,
        deliverable: &mut StageDeliverable,
        spec: &golish_agent_kit::harness::StageSpec,
    ) {
        if spec.min_invocations.is_empty() {
            return;
        }
        let Some(repo) = self.evidence_repo.as_ref() else {
            return;
        };
        let ids = collect_cited_evidence_ids(deliverable);
        if ids.is_empty() {
            return;
        }
        let kinds = match repo.evidence_kinds_for(&ids).await {
            Ok(kinds) => kinds,
            Err(e) => {
                tracing::warn!(
                    target: "harness::submit_tool",
                    error = %e,
                    "evidence kind lookup failed; submit preview keeps model-provided required_checks_done"
                );
                return;
            }
        };
        let added = backfill_required_checks_done_from_kinds(deliverable, spec, &kinds);
        if !added.is_empty() {
            tracing::info!(
                target: "harness::submit_tool",
                checks = ?added,
                "backfilled required_checks_done from cited evidence kinds"
            );
        }
    }

    async fn active_stage_coverage_context(
        &self,
        repo: &Arc<dyn EvidenceLedgerQuery>,
        stage: StageKind,
        organization_id: Option<Uuid>,
    ) -> Result<
        (
            Option<chrono::DateTime<chrono::Utc>>,
            Option<StageAssetWaveView>,
        ),
        String,
    > {
        let operation_id = {
            let Some(src) = self.operation_id_source.as_ref() else {
                return Ok((None, None));
            };
            *src.read().await
        };
        let Some(operation_id) = operation_id else {
            return Ok((None, None));
        };
        let Some((active_stage, started_at)) = repo.operation_stage_started_at(operation_id).await
        else {
            return Ok((None, None));
        };
        if active_stage != stage {
            return Ok((None, None));
        }
        if let Some(organization_id) = organization_id {
            let wave = repo
                .stage_asset_wave_current_running(operation_id, organization_id, stage)
                .await
                .map_err(|error| format!("failed to read current asset wave: {error}"))?;
            if let Some(wave) = wave {
                wave.validate_membership()
                    .map_err(|error| format!("invalid current asset wave: {error}"))?;
                return Ok((Some(wave.started_at), Some(wave)));
            }
        }
        Ok((Some(started_at), None))
    }

    async fn candidate_decision_evidence_rejection(
        &self,
        deliverable: &StageDeliverable,
    ) -> Result<Option<String>, String> {
        let repo = self.evidence_repo.as_ref().ok_or_else(|| {
            "attack_candidate submit preview requires the trusted DB repository".to_string()
        })?;
        let context = golish_core::current_agent_tool_context().ok_or_else(|| {
            "attack_candidate submit preview requires trusted tool context".to_string()
        })?;
        let operation_id = context.operation_id.ok_or_else(|| {
            "attack_candidate submit preview requires operation identity".to_string()
        })?;
        let stage_run_unit_id = context.stage_run_unit_id.ok_or_else(|| {
            "attack_candidate submit preview requires StageRunUnit identity".to_string()
        })?;
        let organization_id = context.organization_id.ok_or_else(|| {
            "attack_candidate submit preview requires organization identity".to_string()
        })?;
        let manifest = repo
            .candidate_manifest_for_unit(operation_id, stage_run_unit_id, organization_id)
            .await
            .map_err(|error| format!("attack_candidate manifest load failed: {error}"))?;
        if manifest.operation_id != operation_id || manifest.organization_id != organization_id {
            return Err(
                "attack_candidate manifest load returned a foreign operation or organization"
                    .to_string(),
            );
        }

        let mut by_key = HashMap::with_capacity(manifest.work_items.len());
        for item in &manifest.work_items {
            if by_key.insert(item.work_item_key.as_str(), item).is_some() {
                return Err(
                    "attack_candidate manifest contains duplicate work-item keys".to_string(),
                );
            }
        }
        for decision in &deliverable.candidate_decisions {
            let Some(item) = by_key.get(decision.work_item_key.as_str()) else {
                continue;
            };
            let frozen = item.evidence_ids.iter().copied().collect::<HashSet<_>>();
            if decision
                .evidence_refs
                .iter()
                .any(|evidence_id| !frozen.contains(evidence_id))
            {
                return Ok(Some(format!(
                    "ATTACK_DECISION_EVIDENCE_UNGROUNDED: work item {} cites evidence outside its frozen manifest",
                    decision.work_item_key
                )));
            }
        }
        if let Err(error) = golish_agent_kit::harness::attack_execution::build_candidate_acceptance(
            &manifest,
            &deliverable.candidate_decisions,
        ) {
            return Ok(Some(format!("{}: {error}", error.code())));
        }
        Ok(None)
    }

    /// Build the gate context for the submit-time preview by projecting the
    /// session's evidence facts into it. Without this the preview runs on an
    /// empty (`default`) context, so a stage with `authoritative_found` (e.g.
    /// target_intel) rejects EVERY `found` coverage cell as "never attempted" —
    /// even when the tool already ran and the fact is in the ledger — which traps
    /// per-org recon sub-agents in an endless resubmit loop. Mirrors the
    /// authoritative-found wiring the main-agent stage-close hook already has.
    ///
    /// `authoritative` (T3, 设计 2026-06-23-submit-preview-authoritative-context):
    /// when true (gray-switch `submit_preview_authoritative_context_enabled()`),
    /// ALSO feed host-aware `asset_types` + dynamic `expected_techniques` so the
    /// preview matches the stage-close gate口径. Passed in (not read from env here)
    /// to keep the method deterministically testable.
    async fn gate_context(
        &self,
        stage: StageKind,
        authoritative: bool,
    ) -> Result<GateContext, String> {
        let active_session_id = if matches!(stage, StageKind::Enumeration | StageKind::VulnTriage) {
            Some(
                self.session_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|session_id| !session_id.is_empty())
                    .ok_or_else(|| {
                        format!(
                            "{} submit preview requires the active non-empty run/session id",
                            stage.as_str()
                        )
                    })?,
            )
        } else {
            self.session_id.as_deref()
        };
        let Some(repo) = self.evidence_repo.as_ref() else {
            if matches!(
                stage,
                StageKind::Enumeration
                    | StageKind::VulnTriage
                    | StageKind::AttackCandidate
                    | StageKind::Verification
                    | StageKind::Reporting
            ) {
                return Err(format!(
                    "{} submit preview requires the trusted DB repository",
                    stage.as_str()
                ));
            }
            return Ok(GateContext::default());
        };
        // (1) command-path ledger facts (DNS/WHOIS/SUBDOMAIN from dig/whois/
        //     subfinder), keyed by chat session.
        let mut facts: Vec<EvidenceFact> = if stage == StageKind::ExternalAttackSurface {
            Vec::new()
        } else {
            match active_session_id {
                Some(sid) => repo.evidence_facts(sid).await,
                None => Vec::new(),
            }
        };
        // (2) org-keyed DB business-table truth (ASN/CT/OSINT) + authoritative
        //     asset axis — ONLY when a bound org is present. Techniques with no
        //     CLI tool (ASN/CT/OSINT) only ever land in `organizations.*`; without
        //     merging coverage_truth here the preview saw only command-path facts
        //     and rejected those cells as "never attempted", trapping per-org
        //     recon sub-agents in a resubmit loop even after enrich landed the
        //     data. Mirrors the main-agent stage-close hook
        //     (`fetch_evidence_facts_for_gate`).
        //
        //     With `org_id == None` the in-scope read + org-presence query fall
        //     back to the WHOLE persistent DB (every target ever seeded, across
        //     runs/orgs) and produce no org-level facts — that would only pollute
        //     the asset axis with unrelated hosts. So skip the projection and keep
        //     the deliverable's self-reported axis (prior behaviour).
        let org_id = match self.org_id_source.as_ref() {
            Some(src) => *src.read().await,
            None => None,
        };
        let trusted_operation_id = golish_core::current_agent_tool_context()
            .and_then(|context| context.operation_id)
            .or(match self.operation_id_source.as_ref() {
                Some(source) => *source.read().await,
                None => None,
            });
        let candidate_work_item_keys = if stage == StageKind::AttackCandidate {
            let context = golish_core::current_agent_tool_context().ok_or_else(|| {
                "attack_candidate submit preview requires trusted tool context".to_string()
            })?;
            let operation_id = context.operation_id.ok_or_else(|| {
                "attack_candidate submit preview requires operation identity".to_string()
            })?;
            let stage_run_unit_id = context.stage_run_unit_id.ok_or_else(|| {
                "attack_candidate submit preview requires StageRunUnit identity".to_string()
            })?;
            let organization_id = context.organization_id.ok_or_else(|| {
                "attack_candidate submit preview requires organization identity".to_string()
            })?;
            repo.candidate_manifest_work_item_keys(operation_id, stage_run_unit_id, organization_id)
                .await
                .map(Some)
                .map_err(|error| format!("attack_candidate manifest load failed: {error}"))?
        } else {
            None
        };
        let verification_truth = if stage == StageKind::Verification {
            let operation_id = golish_core::current_agent_tool_context()
                .and_then(|context| context.operation_id)
                .or(match self.operation_id_source.as_ref() {
                    Some(source) => *source.read().await,
                    None => None,
                })
                .ok_or_else(|| {
                    "verification submit preview requires operation identity".to_string()
                })?;
            let truth = repo
                .verification_truth_for_operation(operation_id)
                .await
                .map_err(|error| format!("verification truth load failed: {error}"))?;
            if truth.as_ref().is_some_and(|truth| {
                truth.authority.operation_id != operation_id
                    || truth
                        .snapshots
                        .iter()
                        .any(|row| row.operation_id != operation_id)
            }) {
                return Err(
                    "verification truth load returned a foreign operation snapshot".to_string(),
                );
            }
            truth
        } else {
            None
        };
        let reporting_truth = if stage == StageKind::Reporting {
            let operation_id = golish_core::current_agent_tool_context()
                .and_then(|context| context.operation_id)
                .or(match self.operation_id_source.as_ref() {
                    Some(source) => *source.read().await,
                    None => None,
                })
                .ok_or_else(|| {
                    "reporting submit preview requires operation identity".to_string()
                })?;
            let truth = repo
                .reporting_truth_for_operation(operation_id)
                .await
                .map_err(|error| format!("reporting truth load failed: {error}"))?;
            if truth
                .as_ref()
                .is_some_and(|truth| truth.operation_id != operation_id)
            {
                return Err(
                    "reporting truth load returned a foreign operation snapshot".to_string()
                );
            }
            truth
        } else {
            None
        };
        if matches!(stage, StageKind::Enumeration | StageKind::VulnTriage) && org_id.is_none() {
            return Err(format!(
                "{} submit preview requires the active bound organization",
                stage.as_str()
            ));
        }
        if stage == StageKind::VulnTriage && trusted_operation_id.is_none() {
            return Err(
                "vuln_triage submit preview requires trusted operation identity".to_string(),
            );
        }
        let stage_spec = load_embedded_stage_spec(stage).ok();
        let (stage_started_at, current_wave) = self
            .active_stage_coverage_context(repo, stage, org_id)
            .await?;
        let current_wave_target_ids = current_wave.as_ref().map(|wave| wave.target_ids.clone());
        let current_wave_asset_values = current_wave.as_ref().map(|wave| wave.asset_values.clone());
        let wave_cutoff = stage_spec
            .as_ref()
            .is_some_and(|spec| spec.asset_wave_barrier)
            .then_some(stage_started_at)
            .flatten();
        let asset_axis_cutoff = stage_asset_axis_cutoff(stage, stage_started_at, wave_cutoff);
        let freshness_cutoff = stage_spec
            .as_ref()
            .is_some_and(|spec| spec.freshness_window)
            .then_some(stage_started_at)
            .flatten();
        if matches!(stage, StageKind::Enumeration | StageKind::VulnTriage)
            && freshness_cutoff.is_none()
        {
            return Err(format!(
                "{} submit preview requires the current stage_started_at freshness cutoff",
                stage.as_str()
            ));
        }
        let mut in_scope_assets: Vec<String> = Vec::new();
        let mut typed_assets: Vec<(String, String)> = Vec::new();
        let mut expected_techniques: Option<Vec<String>> = None;
        let mut source_queries: Vec<SourceQueryFact> = Vec::new();
        let mut web_capable_assets: Vec<String> = Vec::new();
        let mut not_applicable_coverage: Vec<(String, String)> = Vec::new();
        let mut outcome_rows: Vec<TechniqueOutcomeFact> = Vec::new();
        let mut authoritative_coverage_axis = false;
        if let Some(org_id) = org_id {
            if stage == StageKind::ExternalAttackSurface {
                if let (Some(session_id), Some(since)) =
                    (self.session_id.as_deref(), freshness_cutoff)
                {
                    facts.extend(
                        repo.eas_evidence_facts_for_session_org_fresh(session_id, org_id, since)
                            .await,
                    );
                }
            }
            let assets = match current_wave_asset_values.as_ref() {
                Some(assets) => assets.clone(),
                None => match asset_axis_cutoff {
                    Some(cutoff) => {
                        repo.in_scope_assets_created_before(Some(org_id), cutoff)
                            .await
                    }
                    None => repo.in_scope_assets(Some(org_id)).await,
                },
            };
            in_scope_assets = assets;
            if !in_scope_assets.is_empty() {
                if let Some(cutoff) = wave_cutoff {
                    tracing::info!(
                        target: "harness::submit_tool",
                        stage = %stage.as_str(),
                        org_id = %org_id,
                        asset_count = in_scope_assets.len(),
                        cutoff = %cutoff,
                        "using current-wave in-scope assets for submit preview"
                    );
                }
            }
            // (3) T3 · authoritative口径补全: host-aware asset_types + dynamic
            //     expected_techniques (same source as the stage-close gate), so the
            //     preview和close对同一交付物给同一判定（消除预检假 PASS / close
            //     BLOCK 分歧）。Each query fail-safes to empty (prior behaviour).
            //     预检不做 subsidiary-inject（需 engagement threshold，预检 seam 不
            //     持有；authoritative stage-close 仍强制该维）。
            if authoritative {
                typed_assets = repo.in_scope_typed_assets(Some(org_id)).await;
                let current_axis = in_scope_assets
                    .iter()
                    .map(String::as_str)
                    .collect::<HashSet<_>>();
                typed_assets.retain(|(asset, _)| current_axis.contains(asset.as_str()));
                let target_types = repo.in_scope_target_types(Some(org_id)).await;
                expected_techniques = stage_gate_expected_techniques(stage, &target_types);
            }
            if stage == StageKind::TargetIntel {
                let organization_context = TargetIntelOrganizationContext::new(org_id);
                organization_context.inject_asset(&mut in_scope_assets);
                if authoritative {
                    organization_context.inject_type(&mut typed_assets);
                }
                not_applicable_coverage.extend(organization_context.not_applicable_coverage());
            }
            // DB business truth must be projected after the synthetic Target
            // Intel organization row is added. Otherwise an organization-only
            // engagement never queries WHOIS/ASN/OSINT truth at submit time.
            if !in_scope_assets.is_empty() {
                facts.extend(
                    repo.db_truth_facts_with_run_start(
                        Some(org_id),
                        &in_scope_assets,
                        freshness_cutoff,
                    )
                    .await,
                );
            }
            if matches!(stage, StageKind::Enumeration | StageKind::VulnTriage) {
                let snapshot = repo
                    .stage_asset_coverage_for_operation(
                        trusted_operation_id,
                        org_id,
                        stage,
                        active_session_id,
                        freshness_cutoff,
                        current_wave_target_ids.clone(),
                        current_wave_asset_values.clone(),
                    )
                    .await
                    .map_err(|error| {
                        format!(
                            "{} exact-origin coverage snapshot failed: {error}",
                            stage.as_str()
                        )
                    })?;
                let snapshot = snapshot.ok_or_else(|| {
                    format!(
                        "{} exact-origin coverage snapshot is unavailable",
                        stage.as_str()
                    )
                })?;
                (in_scope_assets, typed_assets) = if stage == StageKind::Enumeration {
                    validated_enumeration_axis_from_coverage_snapshot(
                        &snapshot,
                        org_id,
                        active_session_id,
                    )
                } else {
                    validated_exact_web_origin_axis_from_coverage_snapshot(
                        &snapshot,
                        stage,
                        org_id,
                        active_session_id,
                    )
                }
                .map_err(|error| {
                    format!(
                        "{} exact-origin coverage snapshot is invalid: {error}",
                        stage.as_str()
                    )
                })?;
                if stage == StageKind::VulnTriage {
                    not_applicable_coverage.extend(
                        trusted_vuln_surface_not_applicable_from_snapshot(&snapshot).map_err(
                            |error| {
                                format!(
                                    "vuln_triage surface applicability snapshot is invalid: {error}"
                                )
                            },
                        )?,
                    );
                }
                authoritative_coverage_axis = true;
            }
            // (4) #4/E3: **始终**从 technique_outcomes union 进 facts（submit 预检；与
            //     execute.rs/org_gate 同源 dual-read）。additive + fail-safe，无灰度开关。
            //     run_id = chat session；outcome blocked→Error（gate 无 Blocked outcome）。
            if let Some(sid) = active_session_id {
                if stage_accepts_outcome_projection(stage, freshness_cutoff.is_some()) {
                    outcome_rows = if stage == StageKind::VulnTriage {
                        let operation_run_id = trusted_operation_id
                            .expect("Vuln operation identity checked above")
                            .to_string();
                        repo.technique_outcome_facts_fresh_with_evidence_session(
                            org_id,
                            &operation_run_id,
                            sid,
                            freshness_cutoff,
                        )
                        .await
                    } else {
                        repo.technique_outcome_facts_fresh(org_id, sid, freshness_cutoff)
                            .await
                    };
                }
                if stage == StageKind::ExternalAttackSurface {
                    not_applicable_coverage
                        .extend(eas_service_not_applicable_from_port_outcomes(&outcome_rows));
                } else if stage == StageKind::VulnTriage {
                    not_applicable_coverage
                        .extend(vuln_not_applicable_from_outcomes(&outcome_rows));
                }
                if stage_accepts_source_query_completion(stage) {
                    source_queries = repo.source_query_facts(org_id, sid).await;
                }
            }
            // (5) Enumeration IP-web coverage (设计 2026-07-01 §5.3): mirror the
            //     stage-close gate (org_gate) — when the enumeration spec opts into
            //     enum_ip_web_coverage, feed EAS/httpx-proven IP web roots so the
            //     preview holds a web-capable IP to the four content axes instead
            //     of dropping it as not_applicable (which would preview-PASS then
            //     close-BLOCK). Non-enumeration / flag off ⇒ stays empty = None.
            if stage == StageKind::Enumeration
                && load_embedded_stage_spec(stage)
                    .map(|s| s.enum_ip_web_coverage)
                    .unwrap_or(false)
            {
                web_capable_assets = repo.enumeration_web_capable_assets(Some(org_id)).await;
            }
            if stage == StageKind::ExternalAttackSurface {
                // EAS SERVICE is strict per confirmed-open port. PORT empty rows
                // were already converted to SERVICE not_applicable above; DNS/53
                // alone still needs the DB truth/worker terminal path.
                web_capable_assets = repo
                    .eas_web_capable_assets(Some(org_id), freshness_cutoff)
                    .await;
            }
        }
        let eas_origin_barrier = if stage == StageKind::ExternalAttackSurface {
            let organization_id = org_id.ok_or_else(|| {
                "external_attack_surface submit preview requires the active bound organization"
                    .to_string()
            })?;
            let since = freshness_cutoff.ok_or_else(|| {
                "external_attack_surface submit preview requires current stage_started_at for exact Web Origins"
                    .to_string()
            })?;
            let required = repo
                .eas_required_web_origins(organization_id, since, current_wave_target_ids.clone())
                .await
                .map_err(|error| {
                    format!(
                        "external_attack_surface exact-origin denominator query failed: {error}"
                    )
                })?;
            let completed = completed_from_guarded_outcomes(&outcome_rows, &facts);
            Some((required, completed))
        } else {
            None
        };
        // Always apply the stage projection, even without org/session/cutoff. For
        // Enumeration an empty row set deliberately clears legacy/business facts.
        apply_technique_outcome_rows(stage, &mut facts, &outcome_rows);
        // 统一组装入口（设计 2026-06-23-unified-gate-context-builder）。
        let mut builder = GateContextBuilder::new()
            .typed_assets(typed_assets)
            .web_capable_assets(web_capable_assets)
            .not_applicable_coverage(not_applicable_coverage)
            .extend_evidence_facts(facts)
            .extend_source_queries(source_queries)
            .expected_techniques(expected_techniques)
            .candidate_work_item_keys(candidate_work_item_keys)
            .reporting_truth(reporting_truth);
        if stage == StageKind::Verification {
            if let Some(snapshots) = verification_truth {
                builder = builder.verification_truth(Some(snapshots));
            }
        }
        if let Some((required, completed)) = eas_origin_barrier {
            builder = builder.eas_web_origin_barrier(required, completed);
        }
        let context = if authoritative_coverage_axis {
            builder.authoritative_in_scope_assets(Some(in_scope_assets))
        } else {
            builder.in_scope_assets(in_scope_assets)
        }
        .build();
        Ok(context)
    }
}

#[async_trait::async_trait]
impl Tool for SubmitStageDeliverableTool {
    fn name(&self) -> &'static str {
        "submit_stage_deliverable"
    }

    fn description(&self) -> &'static str {
        "Submit the structured StageDeliverable for the CURRENT operation stage. \
         Call this once the stage's required tools have actually run. Pass the real \
         structured data as arguments (NOT a prose description, NOT a JSON string in \
         text) — stage_id and claims. Omit empty optional fields; the server \
         canonicalizes evidence_refs, findings, coverage, skipped_checks, and \
         required_checks_done to empty arrays when absent. stage_run_id is assigned \
         by the server, do not pass it. The \
         deterministic gate validates the stage against DB/ledger truth to \
         advance the stage. This is the ONLY way to complete a stage. Evidence ids \
         are optional internal ledger references: do NOT hunt for them or invent \
         them. If you cite ids anyway, they must be real."
    }

    fn parameters(&self) -> Value {
        // Every nested item is spelled out (not a bare `{"type":"object"}`) so the
        // model fills the EXACT shape instead of guessing — the recurring failure
        // mode was the `skipped_checks[].reason` SkipReason enum (internally tagged
        // by `kind`) and the coverage-cell fields, which an opaque object schema
        // left the model to invent and repeatedly fail on.
        json!({
            "type": "object",
            "properties": {
                "stage_id": {
                    "type": "string",
                    "description": "The current stage id, e.g. \"target_intel\", \"external_attack_surface\". Must equal the active stage."
                },
                "claims": {
                    "type": "array",
                    "description": "Business observations. Do not look up or invent evidence_ids; omit them unless a previous tool/result explicitly gave you a real ledger id. When a claim evidences one of the stage's expected techniques, set `technique` to that REGISTERED id (e.g. GOLISH-INTEL-DNS, WSTG-INPV-05) and use the SAME `subject` string as the matching coverage cell's `asset`. Unregistered technique ids are rejected. Enumeration claim kinds should summarize content mapping, e.g. web_root_enumerated, directories_discovered, api_endpoints_discovered, params_discovered, js_candidates_reviewed.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": { "type": "string", "description": "Observation kind, e.g. \"dns_a_record\", \"subdomain\", \"discovery\"." },
                            "subject": { "type": "string", "description": "What the claim is about (host / URL / asset). Match the coverage cell's `asset` when technique-tagged." },
                            "summary": { "type": "string", "description": "One-line human-readable summary." },
                            "evidence_ids": { "type": "array", "items": { "type": "integer" }, "description": "Optional internal ledger ids. Usually omit; never invent placeholders like 1,2,3." },
                            "technique": { "type": "string", "description": "Optional REGISTERED technique id this claim evidences (omit when none applies)." }
                        },
                        "required": ["kind", "subject", "summary"]
                    }
                },
                "evidence_refs": {
                    "type": "array",
                    "description": "Optional internal ledger ids. Usually omit; the server resolves DB/ledger truth. If you include ids, use REAL ids only — never placeholders.",
                    "items": { "type": "integer" }
                },
                "findings": {
                    "type": "array",
                    "description": "Submit [] for vuln_triage: Nuclei results are observation/evidence only and Candidate reasoning happens later. Recon/discovery and attack_candidate also take NO model-authored findings. Only a persisted legacy Verification contract may accept deliverable findings; V2 findings are created by the server-side CandidateAttempt terminalizer. Disallowed findings are DROPPED.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "finding_id": { "type": "string", "description": "A random UUID v4 for this finding." },
                            "kind": { "type": "string", "description": "Finding kind, e.g. \"open_port\", \"exposed_admin\"." },
                            "subject": { "type": "string", "description": "Affected asset (host / URL)." },
                            "severity": { "type": "string", "enum": ["info", "low", "medium", "high", "critical"], "description": "Finding severity." },
                            "evidence_refs": { "type": "array", "items": { "type": "integer" }, "description": "Optional internal ledger ids. Usually omit unless a real id is explicitly available." },
                            "technique": { "type": "string", "description": "Optional REGISTERED technique id this finding evidences." }
                        },
                        "required": ["finding_id", "kind", "subject", "severity"]
                    }
                },
                "candidate_decisions": {
                    "type": "array",
                    "maxItems": 100,
                    "description": "attack_candidate only: exactly one decision for every server-seeded work_item_key. Never provide operation/scope/org/wave/execution/submission ids or an execution plan; the server derives them. Both candidate and no_candidate decisions require real evidence_refs from the work item.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "work_item_key": { "type": "string", "minLength": 1, "maxLength": 256 },
                            "decision": { "type": "string", "enum": ["candidate", "no_candidate"] },
                            "hypothesis": { "type": "string", "minLength": 1, "maxLength": 4096, "description": "Required only for candidate." },
                            "rationale": { "type": "string", "minLength": 1, "maxLength": 8192 },
                            "technique": { "type": "string", "minLength": 1, "maxLength": 128, "description": "Optional for candidate; if present it must equal the frozen work-item technique." },
                            "evidence_refs": { "type": "array", "minItems": 1, "maxItems": 64, "uniqueItems": true, "items": { "type": "integer", "minimum": 1 } },
                            "no_candidate_reason_code": { "type": "string", "minLength": 1, "maxLength": 64, "pattern": "^[a-z0-9_]+$", "description": "Required only for no_candidate." }
                        },
                        "required": ["work_item_key", "decision", "rationale", "evidence_refs"]
                    }
                },
                "candidate_decision_groups": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 100,
                    "description": "attack_candidate compact form for large manifests. Group only work items that genuinely share one decision and rationale. Select each group with exactly one of: exact work_item_keys, canonical manifest-kind work_item_key_prefixes, or nuclei_template_ids. Template selectors match only exact nuclei_match_v1 template ids in the trusted frozen manifest, so TLS security and metadata classes can be decided without copying every hash key. The unchanged Gate still requires exactly one terminal decision for every exact item. The server supplies each item's frozen evidence ids. Use this instead of candidate_decisions, never together.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "work_item_keys": { "type": "array", "minItems": 1, "maxItems": 100, "uniqueItems": true, "items": { "type": "string", "minLength": 1, "maxLength": 256 } },
                            "work_item_key_prefixes": { "type": "array", "minItems": 1, "maxItems": 3, "uniqueItems": true, "items": { "type": "string", "minLength": 2, "maxLength": 64, "pattern": "^[a-z0-9_]+:$" } },
                            "nuclei_template_ids": { "type": "array", "minItems": 1, "maxItems": 16, "uniqueItems": true, "items": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Za-z0-9._/-]+$" }, "description": "Exact Nuclei template ids present in the frozen manifest, such as weak-cipher-suites or ssl-issuer." },
                            "decision": { "type": "string", "enum": ["candidate", "no_candidate"] },
                            "hypothesis": { "type": "string", "minLength": 1, "maxLength": 4096, "description": "Required only for a candidate group." },
                            "rationale": { "type": "string", "minLength": 1, "maxLength": 8192 },
                            "no_candidate_reason_code": { "type": "string", "minLength": 1, "maxLength": 64, "pattern": "^[a-z0-9_]+$", "description": "Required only for a no_candidate group." }
                        },
                        "required": ["decision", "rationale"]
                    }
                },
                "coverage": {
                    "type": "array",
                    "description": "Coverage matrix for stages whose contract still requires model-authored terminal cells. Call check_stage_asset_coverage before submit. ENUMERATION IS FULLY AUTHORITATIVE: always submit coverage=[]; current-run producer evidence owns found/checked_empty, while current-target blocked evidence is limited to enum_preflight_web_origins on all four axes, route_probe_paths recovery on DIR, and browser_collect_js_api recovery on JS/JSAPI/PARAM. Non-web/rootless hosts are excluded before the exact-origin denominator is built. Enumeration model-authored coverage cannot close a cell. Other DB-truth stages should omit DB-derived found cells and include only contract-permitted terminal exceptions. Stages that run no tools submit []. For non-DB-truth stages, missing expected asset × technique cells fail the gate. EAS example: SERVICE-FINGERPRINT tested_units = open ports fingerprinted and total_units = open ports discovered. Evidence ids are optional internal refs; never invent them. Omit optional fields you do not use and never pass null.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "asset": { "type": "string", "description": "Asset identifier (host / URL). Match a claim's `subject`." },
                            "technique": { "type": "string", "description": "REGISTERED technique id (e.g. GOLISH-INTEL-DNS, WSTG-INPV-05)." },
                            "status": { "type": "string", "enum": ["found", "checked_empty", "blocked", "not_applicable"], "description": "Terminal state for this (asset × technique)." },
                            "evidence_refs": { "type": "array", "items": { "type": "integer" }, "description": "Optional internal ledger ids. Usually omit unless real ids are explicitly available." },
                            "note": { "type": "string", "description": "Required for blocked/not_applicable: why." },
                            "reason_kind": { "type": "string", "enum": ["provider_missing", "credential_missing", "rate_limited", "tool_missing", "out_of_scope", "not_applicable"], "description": "Optional structured reason category for blocked/not_applicable (complements `note`)." },
                            "tested_units": { "type": "integer", "description": "How many enumerated units you actually tested for this asset×technique. For EAS service fingerprinting, this is the number of open ports fingerprinted." },
                            "total_units": { "type": "integer", "description": "Denominator: total enumerated units for this asset×technique. For EAS service fingerprinting, this is the number of discovered open ports." },
                            "sampling_rationale": { "type": "string", "description": "Required when tested_units < total_units: why sampling is justified." }
                        },
                        "required": ["asset", "technique", "status"]
                    }
                },
                "skipped_checks": {
                    "type": "array",
                    "description": "Optional deliberately skipped required checks. Omit unless a required check actually could not run. Scope decisions such as excluding subsidiaries in scoping belong in the scope claim summary, not skipped_checks. \"checked-empty\" is NOT \"unchecked\".",
                    "items": {
                        "type": "object",
                        "properties": {
                            "check": { "type": "string", "description": "Name of the check you skipped." },
                            "reason": {
                                "type": "object",
                                "description": "A SkipReason, internally tagged by `kind`.",
                                "properties": {
                                    "kind": { "type": "string", "enum": ["other", "rate_limited", "scope_restriction", "env_unavailable", "user_requested"], "description": "Which SkipReason variant." },
                                    "explanation": { "type": "string", "description": "kind=other: free-text reason." },
                                    "evidence_ref": { "type": "integer", "description": "kind=other: real evidence-ledger id anchoring the skip." },
                                    "tool": { "type": "string", "description": "kind=rate_limited/env_unavailable: the tool." },
                                    "after_attempts": { "type": "integer", "description": "kind=rate_limited: attempts before giving up." },
                                    "restricted_target": { "type": "string", "description": "kind=scope_restriction: the out-of-scope target." },
                                    "error_chain": { "type": "array", "items": { "type": "string" }, "description": "kind=env_unavailable: the error chain." },
                                    "user_msg_id": { "type": "string", "description": "kind=user_requested: the user message id." }
                                },
                                "required": ["kind"]
                            }
                        },
                        "required": ["check", "reason"]
                    }
                },
                "required_checks_done": {
                    "type": "array",
                    "description": "Optional names of required tools you actually ran (e.g. dns_resolve, http_probe). Omit when the active stage declares no required tool invocations; the server defaults this to [].",
                    "items": { "type": "string" }
                }
            },
            "required": ["stage_id", "claims"]
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        let mut args = canonicalize_model_submit_args(args);
        if let Err(reason) = self.expand_candidate_decision_groups(&mut args).await {
            return Ok(json!({
                "status": "rejected",
                "reason": reason,
            }));
        }
        // Force structured emission: parse the args into the canonical type. A
        // prose / malformed submission is rejected with actionable feedback so
        // the model retries with real fields (immediate-feedback = option 甲).
        let mut deliverable: StageDeliverable = match serde_json::from_value(args) {
            Ok(d) => d,
            Err(e) => {
                return Ok(json!({
                    "status": "rejected",
                    "reason": format!(
                        "could not parse StageDeliverable: {e}. Pass the structured fields \
                         (stage_id, claims[], plus optional evidence_refs[], findings[], \
                         coverage[], candidate_decisions[], skipped_checks[], required_checks_done[]) as tool \
                         arguments — do not describe the JSON in prose."
                    ),
                }));
            }
        };

        let active = *self.active_stage.read().await;
        let mut trusted_capture = None;
        if let Some(kind) = active {
            if deliverable.stage_id != kind.as_str() {
                return Ok(json!({
                    "status": "rejected",
                    "reason": format!(
                        "stage_id '{}' does not match the active stage '{}'",
                        deliverable.stage_id,
                        kind.as_str()
                    ),
                }));
            }
            if kind == StageKind::Enumeration && !deliverable.coverage.is_empty() {
                return Ok(json!({
                    "status": "needs_fix",
                    "reasons": [
                        "Enumeration coverage is fully authoritative. Model-authored found/checked_empty/blocked/not_applicable cells are forbidden; resubmit with coverage: []. Trusted blocked truth comes only from transport preflight or bounded route/browser producer recovery."
                    ],
                    "note": "remove every coverage cell and call submit_stage_deliverable again"
                }));
            }
            // Piece 3 · closeout reconciliation barrier: if this session still has
            // backgrounded scans in flight, don't grade the stage against
            // half-landed evidence. Default behavior is a fast deferral that tells
            // the agent to call wait_for_background_jobs, making the wait/output
            // inspection visible before the next submit.
            if let Some(deferred) = self.reconcile_background_jobs().await {
                return Ok(deferred);
            }
            if kind == StageKind::Cleanup {
                if let Some(reason) = self.cleanup_gate_block_reason().await {
                    return Ok(json!({
                        "status": "needs_fix",
                        "reasons": [reason],
                        "note": "Cleanup is graded only from canonical obligation, absence and residual rows. Inspect/retry cleanup or ask the local operator for a residual waiver."
                    }));
                }
            }
            // 甲 · structural/semantic gate preview (no DB). The authoritative
            // evidence-ledger cross-check runs at stage close in the gate hook.
            if let Ok(spec) = load_embedded_stage_spec(kind) {
                let findings_allowed = self
                    .effective_findings_allowed(kind, spec.findings_allowed)
                    .await?;
                // Recon/discovery stages declare `findings_allowed=false`: their
                // deliverable is observations (`claims`) + a coverage matrix, NOT
                // vulnerabilities. Drop any `findings` a (weak) model dumped here so
                // junk never pollutes the stored deliverable or the stage-close gate;
                // the accept note tells the model to put discoveries in `claims`.
                // Design 2026-06-15-recon-stage-findings-suppression.
                let dropped_findings = drop_disallowed_findings(&mut deliverable, findings_allowed);
                // Project the session's ledger evidence-facts into the gate so an
                // authoritative_found stage credits real `found` cells (the
                // per-org recon "never attempted" loop fix). Empty/no-DB ⇒ default
                // context = prior behaviour. T3: gray-switch also feeds host-aware
                // asset_types + dynamic expected_techniques so the preview matches
                // the stage-close口径 (env GOLISH_SUBMIT_PREVIEW_AUTHORITATIVE_CONTEXT=0 reverts).
                self.backfill_required_checks_done_from_evidence(&mut deliverable, &spec)
                    .await;
                let coordinator_pass_token = spec
                    .specialist
                    .as_ref()
                    .and_then(|_| unique_stage_run_pass_token(&deliverable));
                let is_aggregate_coordinator_closeout = coordinator_pass_token.is_some()
                    && (self.runtime_memory_repo.is_none()
                        || golish_core::current_agent_tool_context().is_some_and(|context| {
                            matches!(context.source, golish_core::events::ToolSource::Main)
                                && context.stage_run_unit_id.is_none()
                                && context.worker_lease.is_none()
                                && context.candidate_attempt.is_none()
                        }));
                // Persist after deterministic server normalization so the
                // immutable row is the exact payload graded at stage close, not
                // an earlier model draft. A Gate BLOCK still retains this row.
                trusted_capture = self
                    .persist_trusted_submission(kind, &mut deliverable, coordinator_pass_token)
                    .await?;
                let authoritative = golish_agent_kit::harness::feature_flags::submit_preview_authoritative_context_enabled();
                if is_aggregate_coordinator_closeout {
                    if trusted_capture.is_none() {
                        if let Ok(json_str) = serde_json::to_string(&deliverable) {
                            *self.last_deliverable.write().await = Some(json_str);
                        }
                    }
                    let fabricated = self.fabricated_refs(&deliverable).await;
                    if fabricated.is_empty() {
                        return Ok(Self::attach_submission_identity(
                            json!({
                                "status": "accepted",
                                "note": "stage_run pass_token captured; the final fan-out closeout gate will recompute it from org_stage_completions."
                            }),
                            trusted_capture.as_ref(),
                        ));
                    }
                    let available = self.available_real_ids().await;
                    return Ok(Self::attach_submission_identity(
                        json!({
                            "status": "needs_fix",
                            "reasons": [format!(
                                "cited evidence ids {fabricated:?} do not exist in the evidence ledger. \
                                 The stage_run pass_token claim itself does not need evidence ids. Remove \
                                 the fabricated ids, or only keep ids you know are real. Available ids \
                                 for debugging: {available:?}."
                            )],
                            "fabricated_evidence_refs": fabricated,
                            "available_evidence_ids": available,
                            "note": "fix these and call submit_stage_deliverable again."
                        }),
                        trusted_capture.as_ref(),
                    ));
                }
                // A trusted coordinator pass token is an aggregate receipt for
                // already-finalized per-org units. It deliberately has no worker
                // lease or StageRunUnit identity of its own, so do not build a
                // unit-scoped preview context (Candidate manifests in particular
                // require one). The final fan-out gate re-derives the token from
                // the durable org_stage_completions and remains authoritative.
                let ctx = match self.gate_context(kind, authoritative).await {
                    Ok(ctx) => ctx,
                    Err(reason) => {
                        return Ok(Self::attach_submission_identity(
                            json!({
                                "status": "needs_fix",
                                "reasons": [reason],
                                "note": "the trusted current-wave context is invalid; repair/reset the wave before resubmitting."
                            }),
                            trusted_capture.as_ref(),
                        ));
                    }
                };
                let result =
                    validate_stage_gate_with_context(&deliverable, &spec, None, None, &ctx);
                // Stash the canonical JSON regardless — the stage-close gate is
                // authoritative; a structural block still informs the agent now.
                if trusted_capture.is_none() {
                    if let Ok(json_str) = serde_json::to_string(&deliverable) {
                        *self.last_deliverable.write().await = Some(json_str);
                    }
                }
                if result.allowed {
                    if kind == StageKind::AttackCandidate {
                        match self
                            .candidate_decision_evidence_rejection(&deliverable)
                            .await
                        {
                            Ok(Some(reason)) => {
                                return Ok(Self::attach_submission_identity(
                                    json!({
                                        "status": "needs_fix",
                                        "reasons": [reason],
                                        "note": "each Candidate decision may cite only evidence frozen into its exact server-seeded work item."
                                    }),
                                    trusted_capture.as_ref(),
                                ));
                            }
                            Ok(None) => {}
                            Err(reason) => {
                                return Ok(Self::attach_submission_identity(
                                    json!({
                                        "status": "needs_fix",
                                        "reasons": [reason],
                                        "note": "the trusted Candidate manifest context is invalid; repair the runtime unit before resubmitting."
                                    }),
                                    trusted_capture.as_ref(),
                                ));
                            }
                        }
                    }
                    if spec.expected_techniques.is_empty() && !deliverable.coverage.is_empty() {
                        let available = self.available_real_ids().await;
                        return Ok(Self::attach_submission_identity(
                            json!({
                                "status": "needs_fix",
                                "reasons": [
                                    "This stage declares NO expected techniques and runs no tools, so it has \
                                     no coverage matrix. Resubmit with coverage: [] (remove the invented \
                                     cells)."
                                ],
                                "available_evidence_ids": available,
                                "note": "fix these and call submit_stage_deliverable again."
                            }),
                            trusted_capture.as_ref(),
                        ));
                    }
                    // P2 · validate-on-submit: structure passing is necessary but
                    // NOT sufficient. Cross-check evidence_refs against the real
                    // ledger NOW so a deliverable citing fabricated ids gets an
                    // immediate, actionable `needs_fix` instead of a misleading
                    // `accepted` (which makes the agent advance before the
                    // stage-close gate blocks it on the same fabrication).
                    let fabricated = self.fabricated_refs(&deliverable).await;
                    if fabricated.is_empty() {
                        let mut note = "structure OK and all cited evidence exists in the ledger; \
                                        DB/ledger truth is resolved at stage close."
                            .to_string();
                        if dropped_findings > 0 {
                            note.push_str(&format!(
                                " NOTE: this stage does not take security findings — \
                                 {dropped_findings} finding(s) you submitted were DROPPED. \
                                 Record discoveries (hosts / services / exposures) as `claims` \
                                 and coverage cells, not `findings`."
                            ));
                        }
                        return Ok(Self::attach_submission_identity(
                            json!({ "status": "accepted", "note": note }),
                            trusted_capture.as_ref(),
                        ));
                    }
                    // 乙 · fabricated ids are still rejected, but ids are not
                    // model-required fields. Prefer omission over another
                    // id-filling retry loop.
                    let available = self.available_real_ids().await;
                    let reason = if available.is_empty() {
                        format!(
                            "cited evidence ids {fabricated:?} do not exist in the evidence ledger. \
                             Evidence ids are optional: remove these id fields and resubmit, or run \
                             the stage's required tools if the underlying DB truth is still missing. \
                             Never invent or copy placeholder ids like 1, 2, 3."
                        )
                    } else {
                        format!(
                            "cited evidence ids {fabricated:?} do not exist in the evidence ledger. \
                             Evidence ids are optional: remove these id fields and let the backend \
                             resolve DB/ledger truth, or cite ONLY ids you know are real. Real ids \
                             recorded for this operation (debug hint, newest first): {available:?}. \
                             Never invent or copy placeholder ids."
                        )
                    };
                    return Ok(Self::attach_submission_identity(
                        json!({
                            "status": "needs_fix",
                            "reasons": [reason],
                            "fabricated_evidence_refs": fabricated,
                            "available_evidence_ids": available,
                            "note": "fix these and call submit_stage_deliverable again."
                        }),
                        trusted_capture.as_ref(),
                    ));
                }
                // Keep real ids available as debug context, but do not turn a
                // structural block into an id-fill exercise. Missing coverage is
                // repaired by running/closing the named DB-truth gaps.
                let available = self.available_real_ids().await;
                let coverage_gap_actions = result
                    .recovery_actions
                    .as_ref()
                    .map(|recovery| recovery.coverage_gap_actions.clone())
                    .unwrap_or_default();
                let mut reasons = result.reasons;
                // No-tool stage trap: a stage with no expected techniques (e.g.
                // scoping / reporting) has NO coverage matrix — but weak models
                // still invent evidence-less coverage cells, which then fail the
                // no-tool stage coverage forever. Point them straight at the
                // fix instead of letting them flail.
                if spec.expected_techniques.is_empty() && !deliverable.coverage.is_empty() {
                    reasons.push(
                        "This stage declares NO expected techniques and runs no tools, so it has \
                         no coverage matrix. Resubmit with coverage: [] (remove the invented \
                         cells)."
                            .to_string(),
                    );
                }
                let mut response = json!({
                    "status": "needs_fix",
                    "reasons": reasons,
                    "available_evidence_ids": available,
                    "note": "fix these and call submit_stage_deliverable again."
                });
                if !coverage_gap_actions.is_empty() {
                    response["coverage_gap_actions"] = json!(coverage_gap_actions);
                }
                return Ok(Self::attach_submission_identity(
                    response,
                    trusted_capture.as_ref(),
                ));
            }
        }

        // No active stage / spec unavailable: still stash; the gate hook decides.
        if active.is_none() {
            if self.runtime_memory_repo.is_some() {
                return Err(anyhow::Error::new(RuntimeMemoryError::IdentityMismatch {
                    code: "missing_active_stage",
                }));
            }
            deliverable.stage_run_id = Uuid::new_v4();
        }
        if trusted_capture.is_none() {
            if let Ok(json_str) = serde_json::to_string(&deliverable) {
                *self.last_deliverable.write().await = Some(json_str);
            }
        }
        Ok(Self::attach_submission_identity(
            json!({ "status": "received" }),
            trusted_capture.as_ref(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_agent_kit::db_traits::{
        CreateRuntimeOperation, CreatedRuntimeOperation, PersistedStageDeliverableSubmission,
        ProjectScopeRegistration,
    };

    type StageHandle = Arc<RwLock<Option<StageKind>>>;
    type SinkHandle = Arc<RwLock<Option<String>>>;

    fn handles() -> (StageHandle, SinkHandle) {
        (Arc::new(RwLock::new(None)), Arc::new(RwLock::new(None)))
    }

    /// A structurally valid `scoping` deliverable that passes the full gate
    /// preview: one evidence-backed claim (non-vacuous), and `scoping` declares
    /// no `min_invocations`, so only the scope check runs as a semantic gate.
    fn valid_scoping_args() -> Value {
        json!({
            "stage_id": "scoping",
            "stage_run_id": "3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
            "claims": [{
                "kind": "scope_confirmed",
                "subject": "example.com",
                "summary": "target confirmed in authorized scope",
                "evidence_ids": [1]
            }],
            "evidence_refs": [1],
            "findings": [],
            "skipped_checks": [],
            "required_checks_done": []
        })
    }

    #[derive(Clone)]
    struct TrustedSubmissionRepo {
        contract: golish_agent_kit::runtime_memory::RuntimeMemoryContract,
        submission_id: Uuid,
        writes: Arc<std::sync::Mutex<Vec<NewStageDeliverableSubmission>>>,
    }

    #[async_trait::async_trait]
    impl RuntimeMemoryRepository for TrustedSubmissionRepo {
        async fn project_scope_register_first_open(
            &self,
            _canonical_path: &str,
            _path_sha256: &str,
        ) -> std::result::Result<ProjectScopeRegistration, RuntimeMemoryError> {
            Err(RuntimeMemoryError::Unavailable)
        }

        async fn project_scope_rename(
            &self,
            _project_scope_id: Uuid,
            _expected_old_path: &str,
            _expected_row_version: i64,
            _new_path: &str,
            _new_path_sha256: &str,
        ) -> std::result::Result<ProjectScopeRegistration, RuntimeMemoryError> {
            Err(RuntimeMemoryError::Unavailable)
        }

        async fn create_runtime_operation(
            &self,
            _input: CreateRuntimeOperation,
        ) -> std::result::Result<CreatedRuntimeOperation, RuntimeMemoryError> {
            Err(RuntimeMemoryError::Unavailable)
        }

        async fn runtime_memory_contract_for_operation(
            &self,
            _operation_id: Uuid,
        ) -> std::result::Result<
            golish_agent_kit::runtime_memory::RuntimeMemoryContract,
            RuntimeMemoryError,
        > {
            Ok(self.contract)
        }

        async fn insert_stage_deliverable_submission(
            &self,
            input: NewStageDeliverableSubmission,
        ) -> std::result::Result<PersistedStageDeliverableSubmission, RuntimeMemoryError> {
            self.writes
                .lock()
                .expect("trusted submission writes lock")
                .push(input.clone());
            Ok(PersistedStageDeliverableSubmission {
                deliverable_submission_id: self.submission_id,
                operation_id: input.operation_id,
                stage_execution_id: input.stage_execution_id,
                stage_run_unit_id: input.stage_run_unit_id,
                worker_run_id: input.worker_run_id,
                organization_id: input.organization_id,
                tool_call_record_id: input.tool_call_record_id,
                tool_request_id: input.tool_request_id,
                stage_kind: input.stage_kind,
                attempt_epoch: input.attempt_epoch,
                lease_token: input.lease_token,
                payload: serde_json::from_str(&input.canonical_deliverable_json)
                    .expect("canonical submission JSON"),
                payload_sha256: input.payload_sha256,
            })
        }
    }

    #[tokio::test]
    async fn scoping_submit_projects_seeded_engagement_org_to_preliminary_identity() {
        let operation_id = Uuid::new_v4();
        let engagement_org_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let tool_call_record_id = Uuid::new_v4();
        let submission_id = Uuid::new_v4();
        let writes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let repo = Arc::new(TrustedSubmissionRepo {
            contract: golish_agent_kit::runtime_memory::RuntimeMemoryContract::DualWriteLegacyRead,
            submission_id,
            writes: Arc::clone(&writes),
        });
        let stage = Arc::new(RwLock::new(Some(StageKind::Scoping)));
        let sink = Arc::new(RwLock::new(None));
        let legacy_sink = sink.clone();
        let operation_source = Arc::new(RwLock::new(Some(operation_id)));
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_operation_id_source(operation_source)
            .with_runtime_memory_repository(repo);
        let captured = tool.captured_submission_handle();
        let context = golish_core::AgentToolContext {
            request_id: "trusted-submit-request".to_string(),
            tool_call_record_id: Some(tool_call_record_id),
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(stage_execution_id),
            stage_run_unit_id: None,
            organization_id: Some(engagement_org_id),
            worker_lease: None,
            candidate_attempt: None,
        };

        let output = golish_core::with_agent_tool_context(
            Some(context),
            tool.execute(valid_scoping_args(), Path::new(".")),
        )
        .await
        .expect("trusted Scoping submission");
        assert_eq!(output["status"], "accepted");
        assert_eq!(
            output["deliverable_submission_id"],
            submission_id.to_string()
        );
        assert_eq!(output["stage_execution_id"], stage_execution_id.to_string());

        {
            let writes = writes.lock().expect("trusted submission writes");
            assert_eq!(writes.len(), 1);
            let write = &writes[0];
            assert_eq!(write.operation_id, operation_id);
            assert_eq!(write.stage_execution_id, stage_execution_id);
            assert_eq!(write.stage_run_unit_id, None);
            assert_eq!(write.organization_id, None);
            assert_eq!(write.tool_call_record_id, tool_call_record_id);
            assert_eq!(write.tool_request_id, "trusted-submit-request");
            assert!(write.canonical_deliverable_json.starts_with('{'));
            assert!(!write.canonical_deliverable_json.contains(": "));
            let canonical_payload =
                serde_json::from_str::<Value>(&write.canonical_deliverable_json)
                    .expect("canonical payload");
            assert_eq!(
                write.canonical_deliverable_json,
                canonical_json(&canonical_payload),
                "canonical payload must remain recursively key-sorted as fields evolve"
            );
            assert_eq!(
                canonical_payload["stage_run_id"],
                stage_execution_id.to_string()
            );
        }

        let captured = captured
            .read()
            .await
            .clone()
            .expect("typed captured submission");
        assert_eq!(captured.deliverable_submission_id, submission_id);
        assert_eq!(captured.stage_execution_id, stage_execution_id);
        assert_eq!(captured.stage_run_unit_id, None);
        assert!(
            legacy_sink.read().await.is_none(),
            "V2 submission must not publish through the shared legacy sink"
        );
    }

    #[tokio::test]
    async fn only_scoping_submission_may_start_without_a_unit() {
        let operation_id = Uuid::new_v4();
        let writes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let repo = Arc::new(TrustedSubmissionRepo {
            contract: golish_agent_kit::runtime_memory::RuntimeMemoryContract::DualWriteV2Preferred,
            submission_id: Uuid::new_v4(),
            writes: Arc::clone(&writes),
        });
        let stage = Arc::new(RwLock::new(Some(StageKind::TargetIntel)));
        let sink = Arc::new(RwLock::new(None));
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))))
            .with_runtime_memory_repository(repo);
        let context = golish_core::AgentToolContext {
            request_id: "missing-unit".to_string(),
            tool_call_record_id: Some(Uuid::new_v4()),
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: None,
            organization_id: None,
            worker_lease: None,
            candidate_attempt: None,
        };
        let args = json!({
            "stage_id": "target_intel",
            "stage_run_id": Uuid::new_v4(),
            "claims": []
        });

        let error =
            golish_core::with_agent_tool_context(Some(context), tool.execute(args, Path::new(".")))
                .await
                .expect_err("post-Scoping submission without unit must fail closed");
        assert!(error.to_string().contains("missing_stage_run_unit"));
        assert!(writes.lock().expect("trusted submission writes").is_empty());
    }

    #[tokio::test]
    async fn coordinator_stage_run_pass_token_skips_per_unit_submission() {
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let root_org_id = Uuid::new_v4();
        let writes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let repo = Arc::new(TrustedSubmissionRepo {
            contract: golish_agent_kit::runtime_memory::RuntimeMemoryContract::DualWriteV2Preferred,
            submission_id: Uuid::new_v4(),
            writes: Arc::clone(&writes),
        });
        let stage = Arc::new(RwLock::new(Some(StageKind::TargetIntel)));
        let legacy_sink = Arc::new(RwLock::new(None));
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&legacy_sink))
            .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))))
            .with_runtime_memory_repository(repo);
        let captured = tool.captured_submission_handle();
        let context = golish_core::AgentToolContext {
            request_id: "coordinator-pass-token".to_string(),
            tool_call_record_id: Some(Uuid::new_v4()),
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(stage_execution_id),
            stage_run_unit_id: None,
            organization_id: Some(root_org_id),
            worker_lease: None,
            candidate_attempt: None,
        };
        let args = json!({
            "stage_id": "target_intel",
            "claims": [
                {
                    "kind": "stage_run_pass_token",
                    "subject": "target_intel",
                    "summary": "deterministic-token"
                },
                {
                    "kind": "model_authored_extra",
                    "subject": "example.com",
                    "summary": "must not enter aggregate closeout",
                    "evidence_ids": [99]
                }
            ],
            "evidence_refs": [99],
            "required_checks_done": ["model_authored_extra"]
        });

        let output =
            golish_core::with_agent_tool_context(Some(context), tool.execute(args, Path::new(".")))
                .await
                .expect("trusted coordinator pass-token closeout");

        assert_eq!(output["status"], "accepted");
        assert!(
            writes.lock().expect("trusted submission writes").is_empty(),
            "aggregate pass-token closeout is not a second per-unit deliverable"
        );
        assert!(captured.read().await.is_none());
        let legacy = legacy_sink
            .read()
            .await
            .clone()
            .expect("aggregate closeout capture");
        let deliverable: StageDeliverable =
            serde_json::from_str(&legacy).expect("aggregate closeout deliverable");
        assert_eq!(deliverable.stage_run_id, stage_execution_id);
        assert_eq!(deliverable.claims.len(), 1);
        assert!(deliverable.evidence_refs.is_empty());
        assert!(deliverable.required_checks_done.is_empty());
        assert_eq!(
            golish_agent_kit::harness::org_gate::extract_pass_token(&deliverable).as_deref(),
            Some("deterministic-token")
        );
    }

    #[tokio::test]
    async fn attack_candidate_coordinator_pass_token_does_not_require_worker_unit_context() {
        let operation_id = Uuid::new_v4();
        let stage_execution_id = Uuid::new_v4();
        let root_org_id = Uuid::new_v4();
        let writes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let repo = Arc::new(TrustedSubmissionRepo {
            contract: golish_agent_kit::runtime_memory::RuntimeMemoryContract::DualWriteV2Preferred,
            submission_id: Uuid::new_v4(),
            writes: Arc::clone(&writes),
        });
        let stage = Arc::new(RwLock::new(Some(StageKind::AttackCandidate)));
        let legacy_sink = Arc::new(RwLock::new(None));
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&legacy_sink))
            .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))))
            .with_evidence_repo(Arc::new(MockLedger::existing(HashSet::new())))
            .with_runtime_memory_repository(repo);
        let context = golish_core::AgentToolContext {
            request_id: "attack-candidate-coordinator-pass-token".to_string(),
            tool_call_record_id: Some(Uuid::new_v4()),
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(stage_execution_id),
            stage_run_unit_id: None,
            organization_id: Some(root_org_id),
            worker_lease: None,
            candidate_attempt: None,
        };

        let output = golish_core::with_agent_tool_context(
            Some(context),
            tool.execute(
                json!({
                    "stage_id": "attack_candidate",
                    "claims": [{
                        "kind": "stage_run_pass_token",
                        "subject": "attack_candidate",
                        "summary": "deterministic-candidate-token"
                    }]
                }),
                Path::new("."),
            ),
        )
        .await
        .expect("candidate coordinator pass-token closeout");

        assert_eq!(output["status"], "accepted", "{output:?}");
        assert!(
            writes.lock().expect("trusted submission writes").is_empty(),
            "aggregate Candidate closeout must not create another per-unit submission"
        );
        assert!(legacy_sink
            .read()
            .await
            .as_deref()
            .is_some_and(|payload| payload.contains("stage_run_pass_token")));
    }

    #[test]
    fn parameters_describe_enumeration_slim_deliverable_contract() {
        let (stage, sink) = handles();
        let tool = SubmitStageDeliverableTool::new(stage, sink);
        let params = tool.parameters().to_string();

        assert!(params.contains("web_root_enumerated"));
        assert!(params.contains("api_endpoints_discovered"));
        assert!(params.contains("check_stage_asset_coverage"));
        assert!(params.contains("ENUMERATION IS FULLY AUTHORITATIVE"));
        assert!(params.contains("enum_preflight_web_origins"));
        assert!(params.contains("route_probe_paths recovery on DIR"));
        assert!(params.contains("browser_collect_js_api recovery on JS/JSAPI/PARAM"));
        assert!(params.contains("coverage=[]"));
    }

    #[test]
    fn parameters_describe_vuln_triage_as_observation_only() {
        let (stage, sink) = handles();
        let schema = SubmitStageDeliverableTool::new(stage, sink).parameters();
        let description = schema["properties"]["findings"]["description"]
            .as_str()
            .expect("findings description");

        assert!(description.contains("Submit [] for vuln_triage"));
        assert!(description.contains("observation/evidence only"));
        assert!(description.contains("CandidateAttempt terminalizer"));
    }

    #[test]
    fn parameters_make_empty_default_fields_optional() {
        let (stage, sink) = handles();
        let tool = SubmitStageDeliverableTool::new(stage, sink);
        let p = tool.parameters();

        assert_eq!(
            p["required"],
            json!(["stage_id", "claims"]),
            "submit schema should only require business-bearing fields"
        );
        assert_eq!(
            p["properties"]["claims"]["items"]["required"],
            json!(["kind", "subject", "summary"]),
            "claim evidence_ids is optional for evidence-free stages such as scoping"
        );
        let skipped_desc = p["properties"]["skipped_checks"]["description"]
            .as_str()
            .expect("skipped_checks description");
        assert!(
            skipped_desc.contains("Scope decisions"),
            "schema should discourage using skipped_checks for normal scope exclusions: {skipped_desc}"
        );
    }

    #[test]
    fn candidate_decision_wire_excludes_trusted_identity_and_legacy_candidate_write() {
        let (stage, sink) = handles();
        let schema = SubmitStageDeliverableTool::new(stage, sink).parameters();
        assert!(schema["properties"].get("candidates").is_none());
        assert_eq!(schema["properties"]["candidate_decisions"]["maxItems"], 100);
        assert_eq!(
            schema["properties"]["candidate_decision_groups"]["maxItems"],
            100
        );
        assert_eq!(
            schema["properties"]["candidate_decision_groups"]["items"]["additionalProperties"],
            false
        );
        assert_eq!(
            schema["properties"]["candidate_decision_groups"]["items"]["properties"]
                ["work_item_key_prefixes"]["maxItems"],
            3
        );
        assert_eq!(
            schema["properties"]["candidate_decision_groups"]["items"]["properties"]
                ["nuclei_template_ids"]["maxItems"],
            16
        );
        assert_eq!(
            schema["properties"]["candidate_decision_groups"]["items"]["required"],
            json!(["decision", "rationale"])
        );
        assert_eq!(
            schema["properties"]["candidate_decisions"]["items"]["additionalProperties"],
            false
        );
        let properties = &schema["properties"]["candidate_decisions"]["items"]["properties"];
        assert_eq!(properties["work_item_key"]["maxLength"], 256);
        assert_eq!(properties["hypothesis"]["maxLength"], 4096);
        assert_eq!(properties["rationale"]["maxLength"], 8192);
        assert_eq!(properties["evidence_refs"]["maxItems"], 64);
        assert_eq!(properties["evidence_refs"]["uniqueItems"], true);
        assert_eq!(
            properties["no_candidate_reason_code"]["pattern"],
            "^[a-z0-9_]+$"
        );
        for forbidden in [
            "operation_id",
            "scope_snapshot_id",
            "organization_id",
            "wave_run_id",
            "wave_unit_id",
            "stage_execution_id",
            "stage_run_unit_id",
            "deliverable_submission_id",
            "execution_plan",
        ] {
            assert!(
                properties.get(forbidden).is_none(),
                "forbidden wire field {forbidden}"
            );
        }
    }

    // §8.1 — prose / malformed args cannot be "described": they fail to parse
    // into StageDeliverable and are rejected with actionable feedback. This is
    // the core property the tool exists to enforce.
    #[tokio::test]
    async fn rejects_prose_for_a_structured_field() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        // `claims` given as a prose string instead of a structured array.
        let args = json!({
            "stage_id": "scoping",
            "stage_run_id": "3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
            "claims": "I generated the deliverable with 5 claims and 3 findings",
            "evidence_refs": [1, 2, 3],
            "findings": []
        });
        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("rejected"));
        // A rejected (unparseable) submission captures nothing.
        assert!(sink.read().await.is_none());
    }

    // §8.1 — stage_id must match the active stage, else rejected (no pollution).
    #[tokio::test]
    async fn rejects_stage_id_mismatch() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let mut args = valid_scoping_args();
        args["stage_id"] = json!("external_attack_surface");
        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("rejected"));
        assert!(out["reason"]
            .as_str()
            .unwrap()
            .contains("does not match the active stage"));
        assert!(sink.read().await.is_none());
    }

    // §8.1 — validate-on-submit PASS branch: a structurally valid deliverable is
    // accepted and captured into the side-channel for the stage-close gate.
    #[tokio::test]
    async fn accepts_valid_deliverable_and_captures_side_channel() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let out = tool
            .execute(valid_scoping_args(), Path::new("/tmp"))
            .await
            .unwrap();
        assert_eq!(out["status"].as_str(), Some("accepted"));

        let captured = sink.read().await.clone().expect("deliverable captured");
        assert!(captured.contains("\"stage_id\":\"scoping\""));
    }

    #[tokio::test]
    async fn accepts_minimal_evidence_free_scoping_deliverable() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let out = tool
            .execute(
                json!({
                    "stage_id": "scoping",
                    "claims": [
                        {
                            "kind": "scope_confirmed",
                            "subject": "1f91fbe0-fcc8-4e3b-848a-0c18cd3fa8de",
                            "summary": "Engagement scope confirmed: 杭州默安科技有限公司 only; subsidiaries are out of scope."
                        },
                        {
                            "kind": "scope_human_approved",
                            "subject": "1f91fbe0-fcc8-4e3b-848a-0c18cd3fa8de",
                            "summary": "Human approved the single-root scope."
                        }
                    ]
                }),
                Path::new("/tmp"),
            )
            .await
            .unwrap();

        assert_eq!(out["status"].as_str(), Some("accepted"), "{out:?}");
        let captured = sink.read().await.clone().expect("deliverable captured");
        let parsed: StageDeliverable = serde_json::from_str(&captured).expect("parse stashed");
        assert_eq!(parsed.claims.len(), 2);
        assert!(parsed
            .claims
            .iter()
            .all(|claim| claim.evidence_ids.is_empty()));
        assert!(parsed.evidence_refs.is_empty());
        assert!(parsed.findings.is_empty());
        assert!(parsed.coverage.is_empty());
        assert!(parsed.skipped_checks.is_empty());
        assert!(parsed.required_checks_done.is_empty());
    }

    #[tokio::test]
    async fn scoping_canonicalizes_legacy_malformed_empty_fields() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let out = tool
            .execute(
                json!({
                    "stage_id": "scoping",
                    "claims": [
                        {
                            "kind": "scope_confirmed",
                            "subject": "1f91fbe0-fcc8-4e3b-848a-0c18cd3fa8de",
                            "summary": "Engagement scope confirmed: 杭州默安科技有限公司 only; subsidiaries are out of scope.",
                            "evidence_ids": null,
                            "technique": null
                        },
                        {
                            "kind": "scope_human_approved",
                            "subject": "1f91fbe0-fcc8-4e3b-848a-0c18cd3fa8de",
                            "summary": "Human approved the single-root scope.",
                            "evidence_ids": null,
                            "technique": null
                        }
                    ],
                    "evidence_refs": null,
                    "findings": null,
                    "coverage": null,
                    "required_checks_done": null,
                    "skipped_checks": [{
                        "check": "recon_discover_subsidiaries",
                        "reason": {
                            "kind": "user_requested",
                            "explanation": "User excluded subsidiaries",
                            "user_msg_id": null
                        }
                    }]
                }),
                Path::new("/tmp"),
            )
            .await
            .unwrap();

        assert_eq!(out["status"].as_str(), Some("accepted"), "{out:?}");
        let captured = sink.read().await.clone().expect("deliverable captured");
        let parsed: StageDeliverable = serde_json::from_str(&captured).expect("parse stashed");
        assert!(parsed.skipped_checks.is_empty());
        assert!(parsed.evidence_refs.is_empty());
        assert!(parsed.claims.iter().all(|claim| claim.technique.is_none()));
    }

    #[tokio::test]
    async fn accepts_stage_run_pass_token_for_specialist_stage_without_evidence() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::TargetIntel);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let out = tool
            .execute(
                json!({
                    "stage_id": "target_intel",
                    "claims": [{
                        "kind": "stage_run_pass_token",
                        "subject": "target_intel",
                        "summary": "abc123",
                        "evidence_ids": []
                    }],
                    "evidence_refs": [],
                    "findings": [],
                    "coverage": [],
                    "skipped_checks": [],
                    "required_checks_done": ["stage_run"]
                }),
                Path::new("/tmp"),
            )
            .await
            .unwrap();

        assert_eq!(out["status"].as_str(), Some("accepted"));
        let captured = sink.read().await.clone().expect("deliverable captured");
        assert!(captured.contains("stage_run_pass_token"));
    }

    // Coverage-matrix cells still parse as StageDeliverable fields, but no-tool
    // stages such as scoping must not invent a matrix.
    #[tokio::test]
    async fn no_tool_stage_rejects_deliverable_with_coverage_cells() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let mut args = valid_scoping_args();
        args["coverage"] = json!([
            { "asset": "api.example.com", "technique": "WSTG-ATHZ-04",
              "status": "found", "evidence_refs": [1] },
            { "asset": "api.example.com", "technique": "WSTG-INPV-05",
              "status": "checked_empty", "evidence_refs": [1], "note": "no injection observed" }
        ]);
        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("needs_fix"));
        let reasons = out["reasons"].as_array().expect("reasons array");
        assert!(
            reasons
                .iter()
                .any(|r| r.as_str().unwrap_or("").contains("coverage: []")),
            "must hint to resubmit with empty coverage: {reasons:?}"
        );
        assert!(sink.read().await.is_some());
    }

    #[tokio::test]
    async fn enumeration_rejects_model_authored_terminal_coverage() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Enumeration);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));
        for status in ["blocked", "not_applicable"] {
            let out = tool
                .execute(
                    json!({
                        "stage_id": "enumeration",
                        "claims": [],
                        "findings": [],
                        "coverage": [{
                            "asset": "https://app.example.com:443",
                            "technique": "GOLISH-ENUM-DIR",
                            "status": status,
                            "note": "model-authored terminal assertion"
                        }]
                    }),
                    Path::new("/tmp"),
                )
                .await
                .unwrap();
            assert_eq!(out["status"], "needs_fix");
            assert!(out["reasons"][0].as_str().unwrap().contains("coverage: []"));
        }
        assert!(sink.read().await.is_none());
    }

    #[tokio::test]
    async fn eas_accepts_surface_claim_without_model_evidence_ids() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::ExternalAttackSurface);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let args = json!({
            "stage_id": "external_attack_surface",
            "claims": [{
                "kind": "http_service",
                "subject": "api.example.com",
                "summary": "HTTP service observed on the active-mapping worklist"
            }],
            "findings": [],
            "coverage": [{
                "asset": "api.example.com",
                "technique": "GOLISH-EAS-LIVENESS",
                "status": "blocked",
                "note": "liveness probe could not run in this test fixture"
            }]
        });

        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("accepted"));
        assert!(out["note"]
            .as_str()
            .unwrap_or("")
            .contains("DB/ledger truth is resolved at stage close"));
        assert!(sink.read().await.is_some());
    }

    // P5 Task 7 · a technique-tagged claim parses, passes the gate preview, and the
    // tag is carried verbatim into the side-channel JSON for the stage-close gate.
    #[tokio::test]
    async fn accepts_technique_tagged_claims_and_captures_them() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let mut args = valid_scoping_args();
        args["claims"][0]["technique"] = json!("GOLISH-INTEL-DNS");
        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("accepted"));

        let captured = sink.read().await.clone().expect("deliverable captured");
        assert!(captured.contains("GOLISH-INTEL-DNS"));
    }

    // P5 Task 7 · an unregistered technique id is rejected at submit time
    // (schema_check runs inside the gate preview), so the model gets an immediate
    // needs_fix naming the bad id instead of a misleading `accepted`.
    #[tokio::test]
    async fn needs_fix_on_unregistered_technique() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let mut args = valid_scoping_args();
        args["claims"][0]["technique"] = json!("GOLISH-INTEL-TYPO");
        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("needs_fix"));
        let reasons = out["reasons"].as_array().expect("reasons array");
        assert!(
            reasons
                .iter()
                .any(|r| r.as_str().unwrap_or("").contains("GOLISH-INTEL-TYPO")),
            "a reason must name the unregistered technique: {reasons:?}"
        );
        // Still stashed — the stage-close gate hook is authoritative.
        assert!(sink.read().await.is_some());
    }

    // §8.1 — validate-on-submit BLOCK branch: a vacuous deliverable (parses fine,
    // matches the stage) fails the gate preview with reasons, yet is still stashed
    // because the stage-close gate hook is authoritative.
    #[tokio::test]
    async fn needs_fix_on_vacuous_deliverable_still_captures() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let args = json!({
            "stage_id": "scoping",
            "stage_run_id": "3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
            "claims": [],
            "evidence_refs": [],
            "findings": []
        });
        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("needs_fix"));
        let reasons = out["reasons"].as_array().expect("reasons array");
        assert!(!reasons.is_empty());
        assert!(sink.read().await.is_some());
    }

    /// Minimal [`EvidenceLedgerQuery`] mock: `existing` is the set that "exists";
    /// `recent` is what `recent_evidence_ids` returns (乙 real-id suggestion).
    struct MockLedger {
        existing: HashSet<i64>,
        recent: Vec<i64>,
        kinds: HashMap<i64, String>,
        facts: Vec<EvidenceFact>,
        source_queries: Vec<SourceQueryFact>,
    }

    impl MockLedger {
        fn existing(ids: HashSet<i64>) -> Self {
            Self {
                existing: ids,
                recent: Vec::new(),
                kinds: HashMap::new(),
                facts: Vec::new(),
                source_queries: Vec::new(),
            }
        }
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for MockLedger {
        async fn existing_evidence_ids(&self, ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(ids
                .iter()
                .copied()
                .filter(|i| self.existing.contains(i))
                .collect())
        }
        async fn recent_evidence_ids(&self, _session_id: &str, _limit: i64) -> Result<Vec<i64>> {
            Ok(self.recent.clone())
        }
        async fn evidence_kinds_for(&self, _ids: &[i64]) -> Result<HashMap<i64, String>> {
            Ok(self.kinds.clone())
        }
        async fn evidence_facts(&self, _session_id: &str) -> Vec<EvidenceFact> {
            self.facts.clone()
        }
        async fn in_scope_assets(&self, _org: Option<uuid::Uuid>) -> Vec<String> {
            let mut assets = self
                .facts
                .iter()
                .map(|fact| fact.asset.clone())
                .collect::<Vec<_>>();
            assets.sort();
            assets.dedup();
            assets
        }
        async fn in_scope_typed_assets(&self, org: Option<uuid::Uuid>) -> Vec<(String, String)> {
            self.in_scope_assets(org)
                .await
                .into_iter()
                .map(|asset| (asset, "domain".to_string()))
                .collect()
        }
        async fn source_query_facts(
            &self,
            _org_id: uuid::Uuid,
            _run_id: &str,
        ) -> Vec<SourceQueryFact> {
            self.source_queries.clone()
        }
    }

    struct VerificationTruthMock {
        truth: Option<golish_agent_kit::harness::attack_execution::VerificationTruthSet>,
    }

    struct ReportingTruthMock {
        truth: Option<golish_agent_kit::harness::ReportingGateTruth>,
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for VerificationTruthMock {
        async fn existing_evidence_ids(&self, _ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(HashSet::new())
        }

        async fn verification_truth_for_operation(
            &self,
            _operation_id: Uuid,
        ) -> Result<Option<golish_agent_kit::harness::attack_execution::VerificationTruthSet>>
        {
            Ok(self.truth.clone())
        }
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for ReportingTruthMock {
        async fn existing_evidence_ids(&self, _ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(HashSet::new())
        }

        async fn reporting_truth_for_operation(
            &self,
            _operation_id: Uuid,
        ) -> Result<Option<golish_agent_kit::harness::ReportingGateTruth>> {
            Ok(self.truth.clone())
        }
    }

    struct CandidateManifestMock {
        manifest: golish_agent_kit::harness::attack_execution::CandidateManifestSnapshot,
        existing: HashSet<i64>,
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for CandidateManifestMock {
        async fn existing_evidence_ids(&self, ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(ids
                .iter()
                .copied()
                .filter(|id| self.existing.contains(id))
                .collect())
        }

        async fn candidate_manifest_work_item_keys(
            &self,
            _operation_id: Uuid,
            _stage_run_unit_id: Uuid,
            _organization_id: Uuid,
        ) -> Result<Vec<String>> {
            Ok(self
                .manifest
                .work_items
                .iter()
                .map(|item| item.work_item_key.clone())
                .collect())
        }

        async fn candidate_manifest_for_unit(
            &self,
            _operation_id: Uuid,
            _stage_run_unit_id: Uuid,
            _organization_id: Uuid,
        ) -> Result<golish_agent_kit::harness::attack_execution::CandidateManifestSnapshot>
        {
            Ok(self.manifest.clone())
        }
    }

    fn candidate_manifest_fixture(
        operation_id: Uuid,
        organization_id: Uuid,
        work_item_key: &str,
    ) -> golish_agent_kit::harness::attack_execution::CandidateManifestSnapshot {
        use golish_agent_kit::harness::attack_execution::{
            CandidateManifestSnapshot, CandidateManifestWorkItem,
            CANDIDATE_SURFACE_ANALYSIS_TECHNIQUE,
        };

        let target_id = Uuid::new_v4();
        let observation = json!({
            "schema": "surface_analysis_v1",
            "target_id": target_id,
            "target_identity": {
                "type": "url",
                "value": "https://youchuang7.com:443",
                "sha256": "sha256:b950",
            },
            "formulaic_coverage": [],
            "upstream_query_required": true,
        });
        CandidateManifestSnapshot {
            operation_id,
            scope_snapshot_id: Uuid::new_v4(),
            wave_run_id: Uuid::new_v4(),
            wave_unit_id: Uuid::new_v4(),
            organization_id,
            manifest_hash: "sha256:manifest".to_string(),
            work_items: vec![CandidateManifestWorkItem {
                work_item_id: Uuid::new_v4(),
                work_item_key: work_item_key.to_string(),
                target_live_id: Some(target_id),
                target_type_at_time: "url".to_string(),
                target_value_at_time: "https://youchuang7.com:443".to_string(),
                target_identity_hash: "sha256:b950".to_string(),
                technique: CANDIDATE_SURFACE_ANALYSIS_TECHNIQUE.to_string(),
                source_fact_delta_id: None,
                delta_kind: None,
                observation_kind: "surface_analysis_v1".to_string(),
                allowed_techniques: vec!["WSTG-INFO".to_string()],
                enrichment_required: false,
                observation_hash: "sha256:observation".to_string(),
                observation,
                evidence_ids: (41..=50).collect(),
            }],
        }
    }

    #[tokio::test]
    async fn candidate_decision_groups_expand_exact_keys_with_server_frozen_evidence() {
        let operation_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let stage_run_unit_id = Uuid::new_v4();
        let work_item_key = "surface_analysis:sha256:b950";
        let manifest = candidate_manifest_fixture(operation_id, organization_id, work_item_key);
        let tool = SubmitStageDeliverableTool::new(handles().0, handles().1).with_evidence_repo(
            Arc::new(CandidateManifestMock {
                manifest,
                existing: (41..=50).collect(),
            }),
        );
        let context = golish_core::AgentToolContext {
            request_id: "candidate-groups".to_string(),
            tool_call_record_id: None,
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: Some(stage_run_unit_id),
            organization_id: Some(organization_id),
            worker_lease: None,
            candidate_attempt: None,
        };
        let mut args = json!({
            "stage_id": "attack_candidate",
            "claims": [],
            "candidate_decision_groups": [{
                "work_item_keys": [work_item_key],
                "decision": "no_candidate",
                "rationale": "Context-only cell has no typed observation.",
                "no_candidate_reason_code": "typed_observation_required"
            }]
        });

        golish_core::with_agent_tool_context(
            Some(context.clone()),
            tool.expand_candidate_decision_groups(&mut args),
        )
        .await
        .expect("trusted exact-key group expands");

        assert!(args.get("candidate_decision_groups").is_none());
        assert_eq!(args["candidate_decisions"].as_array().unwrap().len(), 1);
        assert_eq!(
            args["candidate_decisions"][0]["evidence_refs"],
            json!([41, 42, 43, 44, 45, 46, 47, 48, 49, 50])
        );
        assert_eq!(
            args["candidate_decisions"][0]["work_item_key"],
            work_item_key
        );
    }

    #[tokio::test]
    async fn candidate_decision_groups_reject_duplicate_or_unknown_keys() {
        let operation_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let stage_run_unit_id = Uuid::new_v4();
        let work_item_key = "surface_analysis:sha256:b950";
        let manifest = candidate_manifest_fixture(operation_id, organization_id, work_item_key);
        let tool = SubmitStageDeliverableTool::new(handles().0, handles().1).with_evidence_repo(
            Arc::new(CandidateManifestMock {
                manifest,
                existing: (41..=50).collect(),
            }),
        );
        let context = golish_core::AgentToolContext {
            request_id: "candidate-groups-invalid".to_string(),
            tool_call_record_id: None,
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: Some(stage_run_unit_id),
            organization_id: Some(organization_id),
            worker_lease: None,
            candidate_attempt: None,
        };
        let mut duplicate = json!({
            "stage_id": "attack_candidate",
            "claims": [],
            "candidate_decision_groups": [{
                "work_item_keys": [work_item_key, work_item_key],
                "decision": "no_candidate",
                "rationale": "Context only.",
                "no_candidate_reason_code": "typed_observation_required"
            }]
        });
        let error = golish_core::with_agent_tool_context(
            Some(context.clone()),
            tool.expand_candidate_decision_groups(&mut duplicate),
        )
        .await
        .expect_err("duplicate key must fail closed");
        assert!(error.contains("more than one decision group"));

        let mut unknown = json!({
            "stage_id": "attack_candidate",
            "claims": [],
            "candidate_decision_groups": [{
                "work_item_keys": ["surface_analysis:sha256:unknown"],
                "decision": "no_candidate",
                "rationale": "Context only.",
                "no_candidate_reason_code": "typed_observation_required"
            }]
        });
        let error = golish_core::with_agent_tool_context(
            Some(context),
            tool.expand_candidate_decision_groups(&mut unknown),
        )
        .await
        .expect_err("unknown key must fail closed");
        assert!(error.contains("unknown work item"));
    }

    #[tokio::test]
    async fn candidate_decision_groups_expand_manifest_kind_prefixes_exactly() {
        let operation_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let stage_run_unit_id = Uuid::new_v4();
        let mut manifest = candidate_manifest_fixture(
            operation_id,
            organization_id,
            "surface_analysis:sha256:first",
        );
        let mut second = manifest.work_items[0].clone();
        second.work_item_id = Uuid::new_v4();
        second.work_item_key = "surface_analysis:sha256:second".to_string();
        second.evidence_ids = vec![51];
        manifest.work_items.push(second);
        let tool = SubmitStageDeliverableTool::new(handles().0, handles().1).with_evidence_repo(
            Arc::new(CandidateManifestMock {
                manifest,
                existing: (41..=51).collect(),
            }),
        );
        let context = golish_core::AgentToolContext {
            request_id: "candidate-prefix-groups".to_string(),
            tool_call_record_id: None,
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: Some(stage_run_unit_id),
            organization_id: Some(organization_id),
            worker_lease: None,
            candidate_attempt: None,
        };
        let mut args = json!({
            "stage_id": "attack_candidate",
            "claims": [],
            "candidate_decision_groups": [{
                "work_item_key_prefixes": ["surface_analysis:"],
                "decision": "no_candidate",
                "rationale": "Context-only observations have no exact verifier input.",
                "no_candidate_reason_code": "typed_observation_required"
            }]
        });

        golish_core::with_agent_tool_context(
            Some(context.clone()),
            tool.expand_candidate_decision_groups(&mut args),
        )
        .await
        .expect("manifest-kind prefix expands through the trusted frozen manifest");

        let decisions = args["candidate_decisions"].as_array().unwrap();
        assert_eq!(decisions.len(), 2);
        assert_eq!(
            decisions
                .iter()
                .map(|decision| decision["work_item_key"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "surface_analysis:sha256:first",
                "surface_analysis:sha256:second"
            ]
        );
        assert_eq!(
            decisions[0]["evidence_refs"],
            json!([41, 42, 43, 44, 45, 46, 47, 48, 49, 50])
        );
        assert_eq!(decisions[1]["evidence_refs"], json!([51]));

        let mut combined_candidate = json!({
            "stage_id": "attack_candidate",
            "claims": [],
            "candidate_decision_groups": [{
                "nuclei_template_ids": ["weak-cipher-suites", "ssl-issuer"],
                "decision": "candidate",
                "hypothesis": "One generic TLS hypothesis.",
                "rationale": "One rationale cannot preserve two template identities."
            }]
        });
        let error = golish_core::with_agent_tool_context(
            Some(context),
            tool.expand_candidate_decision_groups(&mut combined_candidate),
        )
        .await
        .expect_err("candidate template groups must remain template-specific");
        assert!(error.contains("exactly one template id"));
    }

    #[tokio::test]
    async fn candidate_decision_groups_expand_exact_nuclei_template_ids() {
        let operation_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let stage_run_unit_id = Uuid::new_v4();
        let mut manifest = candidate_manifest_fixture(
            operation_id,
            organization_id,
            "scanner_observation:sha256:weak",
        );
        manifest.work_items[0].observation_kind = "nuclei_match_v1".to_string();
        manifest.work_items[0].technique = "WSTG-CRYP-03".to_string();
        manifest.work_items[0].observation = json!({"template_id": "weak-cipher-suites"});
        let mut metadata = manifest.work_items[0].clone();
        metadata.work_item_id = Uuid::new_v4();
        metadata.work_item_key = "scanner_observation:sha256:issuer".to_string();
        metadata.observation = json!({"template_id": "ssl-issuer"});
        metadata.evidence_ids = vec![51];
        manifest.work_items.push(metadata);
        let tool = SubmitStageDeliverableTool::new(handles().0, handles().1).with_evidence_repo(
            Arc::new(CandidateManifestMock {
                manifest,
                existing: (41..=51).collect(),
            }),
        );
        let context = golish_core::AgentToolContext {
            request_id: "candidate-template-groups".to_string(),
            tool_call_record_id: None,
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: Some(stage_run_unit_id),
            organization_id: Some(organization_id),
            worker_lease: None,
            candidate_attempt: None,
        };
        let mut args = json!({
            "stage_id": "attack_candidate",
            "claims": [],
            "candidate_decision_groups": [{
                "nuclei_template_ids": ["weak-cipher-suites"],
                "decision": "candidate",
                "hypothesis": "The exact TLS weakness remains reproducible.",
                "rationale": "Frozen Nuclei evidence supports exact safe replay."
            }, {
                "nuclei_template_ids": ["ssl-issuer"],
                "decision": "no_candidate",
                "rationale": "Issuer identity is inventory context only.",
                "no_candidate_reason_code": "tls_metadata_only"
            }]
        });

        golish_core::with_agent_tool_context(
            Some(context),
            tool.expand_candidate_decision_groups(&mut args),
        )
        .await
        .expect("trusted Nuclei template selectors expand exact frozen items");

        let decisions = args["candidate_decisions"].as_array().unwrap();
        assert_eq!(decisions.len(), 2);
        assert_eq!(
            decisions
                .iter()
                .map(|decision| decision["work_item_key"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "scanner_observation:sha256:weak",
                "scanner_observation:sha256:issuer"
            ]
        );
        assert_eq!(
            decisions[0]["evidence_refs"],
            json!([41, 42, 43, 44, 45, 46, 47, 48, 49, 50])
        );
        assert_eq!(decisions[1]["evidence_refs"], json!([51]));
    }

    #[tokio::test]
    async fn candidate_submit_preview_rejects_duplicate_semantic_identity() {
        let operation_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let stage_run_unit_id = Uuid::new_v4();
        let target_id = Uuid::new_v4();
        let mut manifest = candidate_manifest_fixture(
            operation_id,
            organization_id,
            "scanner_observation:sha256:weak",
        );
        manifest.work_items[0].target_live_id = Some(target_id);
        manifest.work_items[0].technique = "WSTG-CRYP-03".to_string();
        manifest.work_items[0].observation_kind = "nuclei_match_v1".to_string();
        manifest.work_items[0].allowed_techniques = vec!["WSTG-CRYP-03".to_string()];
        manifest.work_items[0].observation = json!({
            "schema": "nuclei_match_v1",
            "source_mode": "general",
            "target_id": target_id,
            "matched_url": "https://youchuang7.com:443/",
            "template_id": "weak-cipher-suites",
            "technique": "WSTG-CRYP-03"
        });
        let observation_hash = |observation: &Value| {
            let digest = Sha256::digest(
                serde_json::to_vec(observation).expect("serialize test observation"),
            );
            format!(
                "sha256:{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )
        };
        manifest.work_items[0].observation_hash =
            observation_hash(&manifest.work_items[0].observation);
        let mut second = manifest.work_items[0].clone();
        second.work_item_id = Uuid::new_v4();
        second.work_item_key = "scanner_observation:sha256:deprecated".to_string();
        second.observation["template_id"] = json!("deprecated-tls");
        second.observation_hash = observation_hash(&second.observation);
        second.evidence_ids = vec![51];
        manifest.work_items.push(second);
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::AttackCandidate);
        let tool = SubmitStageDeliverableTool::new(stage, sink).with_evidence_repo(Arc::new(
            CandidateManifestMock {
                manifest,
                existing: (41..=51).collect(),
            },
        ));
        let context = golish_core::AgentToolContext {
            request_id: "candidate-duplicate-semantic-identity".to_string(),
            tool_call_record_id: None,
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: Some(stage_run_unit_id),
            organization_id: Some(organization_id),
            worker_lease: None,
            candidate_attempt: None,
        };
        let output = golish_core::with_agent_tool_context(
            Some(context),
            tool.execute(
                json!({
                    "stage_id": "attack_candidate",
                    "claims": [{
                        "kind": "candidate_synthesis",
                        "subject": "https://youchuang7.com:443",
                        "summary": "TLS decisions synthesized"
                    }],
                    "candidate_decision_groups": [{
                        "work_item_keys": ["scanner_observation:sha256:weak"],
                        "decision": "candidate",
                        "hypothesis": "The exact TLS configuration remains weak",
                        "rationale": "Frozen weak-cipher evidence supports replay"
                    }, {
                        "work_item_keys": ["scanner_observation:sha256:deprecated"],
                        "decision": "candidate",
                        "hypothesis": "The exact TLS configuration remains weak",
                        "rationale": "Frozen deprecated-TLS evidence supports replay"
                    }]
                }),
                Path::new("."),
            ),
        )
        .await
        .expect("duplicate semantic identity returns repair feedback");

        assert_eq!(output["status"], "needs_fix", "output={output}");
        assert!(
            output["reasons"]
                .to_string()
                .contains("ATTACK_CANDIDATE_DUPLICATE_IDENTITY"),
            "output={output}"
        );
    }

    #[tokio::test]
    async fn candidate_decision_groups_reject_mixed_or_noncanonical_prefix_selectors() {
        let operation_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let stage_run_unit_id = Uuid::new_v4();
        let work_item_key = "surface_analysis:sha256:b950";
        let manifest = candidate_manifest_fixture(operation_id, organization_id, work_item_key);
        let tool = SubmitStageDeliverableTool::new(handles().0, handles().1).with_evidence_repo(
            Arc::new(CandidateManifestMock {
                manifest,
                existing: (41..=50).collect(),
            }),
        );
        let context = golish_core::AgentToolContext {
            request_id: "candidate-prefix-groups-invalid".to_string(),
            tool_call_record_id: None,
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: Some(stage_run_unit_id),
            organization_id: Some(organization_id),
            worker_lease: None,
            candidate_attempt: None,
        };
        let mut mixed = json!({
            "stage_id": "attack_candidate",
            "claims": [],
            "candidate_decision_groups": [{
                "work_item_keys": [work_item_key],
                "work_item_key_prefixes": ["surface_analysis:"],
                "decision": "no_candidate",
                "rationale": "Context only.",
                "no_candidate_reason_code": "typed_observation_required"
            }]
        });
        let error = golish_core::with_agent_tool_context(
            Some(context.clone()),
            tool.expand_candidate_decision_groups(&mut mixed),
        )
        .await
        .expect_err("mixed exact-key and prefix selectors must fail closed");
        assert!(error.contains("exactly one selector"));

        let mut noncanonical = json!({
            "stage_id": "attack_candidate",
            "claims": [],
            "candidate_decision_groups": [{
                "work_item_key_prefixes": ["surface_analysis:sha256:"],
                "decision": "no_candidate",
                "rationale": "Context only.",
                "no_candidate_reason_code": "typed_observation_required"
            }]
        });
        let error = golish_core::with_agent_tool_context(
            Some(context),
            tool.expand_candidate_decision_groups(&mut noncanonical),
        )
        .await
        .expect_err("partial hash prefixes must not become selector authority");
        assert!(error.contains("canonical manifest-kind prefix"));
    }

    #[tokio::test]
    async fn candidate_submit_rejects_real_ledger_evidence_outside_frozen_manifest() {
        let operation_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let stage_run_unit_id = Uuid::new_v4();
        let work_item_key = "surface_analysis:sha256:b950";
        let manifest = candidate_manifest_fixture(operation_id, organization_id, work_item_key);
        let existing = (20..=50).collect::<HashSet<_>>();
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::AttackCandidate);
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(CandidateManifestMock { manifest, existing }));
        let context = golish_core::AgentToolContext {
            request_id: "candidate-outside-manifest".to_string(),
            tool_call_record_id: None,
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: Some(stage_run_unit_id),
            organization_id: Some(organization_id),
            worker_lease: None,
            candidate_attempt: None,
        };
        let output = golish_core::with_agent_tool_context(
            Some(context),
            tool.execute(
                json!({
                    "stage_id": "attack_candidate",
                    "claims": [{
                        "kind": "candidate_selected",
                        "subject": "https://youchuang7.com:443",
                        "summary": "README.md information disclosure candidate"
                    }],
                    "candidate_decisions": [{
                        "work_item_key": work_item_key,
                        "decision": "candidate",
                        "hypothesis": "README.md may disclose deployment information",
                        "rationale": "Evidence 20 records the exposed README.md",
                        "technique": "WSTG-INFO",
                        "evidence_refs": [41,42,43,44,45,46,47,48,49,50,20]
                    }]
                }),
                Path::new("."),
            ),
        )
        .await
        .expect("Candidate submission returns actionable feedback");

        assert_eq!(output["status"], "needs_fix", "output={output}");
        assert!(
            output["reasons"]
                .to_string()
                .contains("ATTACK_DECISION_EVIDENCE_UNGROUNDED"),
            "output={output}"
        );
    }

    fn valid_reporting_truth(operation_id: Uuid) -> golish_agent_kit::harness::ReportingGateTruth {
        let revision_id = Uuid::new_v4();
        golish_agent_kit::harness::ReportingGateTruth {
            operation_id,
            report_id: Uuid::new_v4(),
            current_revision_id: revision_id,
            revision_id,
            validation_status: "validated".to_string(),
            publication_status: "unpublished".to_string(),
            stored_source_set_hash: "a".repeat(64),
            current_source_set_hash: "a".repeat(64),
            source_snapshot_exact: true,
            claims_citations_valid: true,
            validation_attestation_valid: true,
            cleanup_closeout_valid: true,
        }
    }

    #[tokio::test]
    async fn reporting_preview_carries_current_db_truth() {
        let operation_id = Uuid::new_v4();
        let truth = valid_reporting_truth(operation_id);
        let (stage, sink) = handles();
        let tool = SubmitStageDeliverableTool::new(stage, sink).with_evidence_repo(Arc::new(
            ReportingTruthMock {
                truth: Some(truth.clone()),
            },
        ));
        let context = golish_core::AgentToolContext {
            request_id: "reporting-preview".to_string(),
            tool_call_record_id: None,
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: None,
            organization_id: None,
            worker_lease: None,
            candidate_attempt: None,
        };

        let gate_context = golish_core::with_agent_tool_context(
            Some(context),
            tool.gate_context(StageKind::Reporting, true),
        )
        .await
        .expect("reporting DB truth loads");
        assert_eq!(gate_context.reporting_truth, Some(truth));
    }

    #[tokio::test]
    async fn reporting_preview_rejects_foreign_operation_truth() {
        let operation_id = Uuid::new_v4();
        let (stage, sink) = handles();
        let tool = SubmitStageDeliverableTool::new(stage, sink).with_evidence_repo(Arc::new(
            ReportingTruthMock {
                truth: Some(valid_reporting_truth(Uuid::new_v4())),
            },
        ));
        let context = golish_core::AgentToolContext {
            request_id: "reporting-preview-foreign".to_string(),
            tool_call_record_id: None,
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: None,
            organization_id: None,
            worker_lease: None,
            candidate_attempt: None,
        };

        let error = golish_core::with_agent_tool_context(
            Some(context),
            tool.gate_context(StageKind::Reporting, true),
        )
        .await
        .expect_err("foreign Reporting truth must fail closed");
        assert!(error.contains("foreign operation"), "error={error}");
    }

    #[tokio::test]
    async fn verification_preview_marks_v2_empty_snapshot_as_required_truth() {
        let operation_id = Uuid::new_v4();
        let (stage, sink) = handles();
        let tool = SubmitStageDeliverableTool::new(stage, sink).with_evidence_repo(Arc::new(
            VerificationTruthMock {
                truth: Some(
                    golish_agent_kit::harness::attack_execution::VerificationTruthSet {
                        authority: golish_agent_kit::harness::attack_execution::VerificationTruthAuthority {
                            operation_id,
                            scope_snapshot_id: Uuid::new_v4(),
                            wave_run_id: Uuid::new_v4(),
                            expected_units: vec![golish_agent_kit::harness::attack_execution::VerificationUnitAuthority {
                                wave_unit_id: Uuid::new_v4(),
                                organization_id: Uuid::new_v4(),
                            }],
                        },
                        snapshots: Vec::new(),
                    },
                ),
            },
        ));
        let context = golish_core::AgentToolContext {
            request_id: "verification-preview".to_string(),
            tool_call_record_id: None,
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: None,
            organization_id: None,
            worker_lease: None,
            candidate_attempt: None,
        };
        let gate_context = golish_core::with_agent_tool_context(
            Some(context),
            tool.gate_context(StageKind::Verification, true),
        )
        .await
        .expect("V2 query succeeded even though the exact snapshot set is empty");
        assert!(gate_context.verification_truth_required);
        assert_eq!(
            gate_context
                .verification_truth_snapshots
                .as_ref()
                .map(|truth| truth.snapshots.len()),
            Some(0)
        );
    }

    #[tokio::test]
    async fn verification_preview_rejects_foreign_operation_snapshot() {
        use golish_agent_kit::harness::attack_execution::VerificationTruthSnapshot;

        let operation_id = Uuid::new_v4();
        let foreign = VerificationTruthSnapshot {
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            wave_run_id: Uuid::new_v4(),
            wave_unit_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            review_closed: true,
            pending_work_items: 0,
            approved_ever: 0,
            attempts: Vec::new(),
            residual_risks: Vec::new(),
        };
        let (stage, sink) = handles();
        let tool = SubmitStageDeliverableTool::new(stage, sink).with_evidence_repo(Arc::new(
            VerificationTruthMock {
                truth: Some(
                    golish_agent_kit::harness::attack_execution::VerificationTruthSet {
                        authority: golish_agent_kit::harness::attack_execution::VerificationTruthAuthority {
                            operation_id: foreign.operation_id,
                            scope_snapshot_id: foreign.scope_snapshot_id,
                            wave_run_id: foreign.wave_run_id,
                            expected_units: vec![golish_agent_kit::harness::attack_execution::VerificationUnitAuthority {
                                wave_unit_id: foreign.wave_unit_id,
                                organization_id: foreign.organization_id,
                            }],
                        },
                        snapshots: vec![foreign],
                    },
                ),
            },
        ));
        let context = golish_core::AgentToolContext {
            request_id: "verification-foreign-preview".to_string(),
            tool_call_record_id: None,
            tool_name: "submit_stage_deliverable".to_string(),
            source: golish_core::events::ToolSource::Main,
            operation_id: Some(operation_id),
            stage_execution_id: Some(Uuid::new_v4()),
            stage_run_unit_id: None,
            organization_id: None,
            worker_lease: None,
            candidate_attempt: None,
        };
        let error = golish_core::with_agent_tool_context(
            Some(context),
            tool.gate_context(StageKind::Verification, true),
        )
        .await
        .expect_err("foreign DB truth must fail closed before Gate evaluation");
        assert!(error.contains("foreign operation snapshot"));
    }

    #[test]
    fn backfills_required_checks_done_from_cited_http_probe_evidence() {
        let mut spec =
            load_embedded_stage_spec(StageKind::ExternalAttackSurface).expect("EAS spec loads");
        spec.min_invocations.insert("http_probe".to_string(), 1);
        let mut deliverable = StageDeliverable {
            stage_id: StageKind::ExternalAttackSurface.as_str().to_string(),
            stage_run_id: uuid::Uuid::new_v4(),
            claims: vec![],
            evidence_refs: vec![golish_pentest::evidence_ledger::EvidenceAuditId::new(42)],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
            coverage: vec![],
            candidates: vec![],
            candidate_decisions: vec![],
        };
        let added = backfill_required_checks_done_from_kinds(
            &mut deliverable,
            &spec,
            &HashMap::from([(42, "http_probe".to_string())]),
        );

        assert_eq!(added, vec!["http_probe"]);
        assert_eq!(deliverable.required_checks_done, vec!["http_probe"]);

        let added_again = backfill_required_checks_done_from_kinds(
            &mut deliverable,
            &spec,
            &HashMap::from([(42, "http_probe".to_string())]),
        );
        assert!(
            added_again.is_empty(),
            "backfill must not duplicate required_checks_done"
        );
    }

    #[test]
    fn backfills_required_checks_done_from_claim_evidence_ids() {
        let mut spec =
            load_embedded_stage_spec(StageKind::ExternalAttackSurface).expect("EAS spec loads");
        spec.min_invocations.insert("http_probe".to_string(), 1);
        let mut deliverable = StageDeliverable {
            stage_id: StageKind::ExternalAttackSurface.as_str().to_string(),
            stage_run_id: uuid::Uuid::new_v4(),
            claims: vec![golish_agent_kit::harness::StageClaim {
                kind: "http_probe".to_string(),
                subject: "https://example.com".to_string(),
                summary: "httpx returned a live host".to_string(),
                evidence_ids: vec![golish_pentest::evidence_ledger::EvidenceAuditId::new(42)],
                technique: None,
            }],
            evidence_refs: vec![],
            skipped_checks: vec![],
            findings: vec![],
            required_checks_done: vec![],
            coverage: vec![],
            candidates: vec![],
            candidate_decisions: vec![],
        };
        let added = backfill_required_checks_done_from_kinds(
            &mut deliverable,
            &spec,
            &HashMap::from([(42, "http_probe".to_string())]),
        );

        assert_eq!(added, vec!["http_probe"]);
        assert_eq!(deliverable.required_checks_done, vec!["http_probe"]);
    }

    #[test]
    fn required_check_done_mentions_match_tokens_not_substrings() {
        assert!(required_check_done_mentions(
            "http_probe done",
            "http_probe"
        ));
        assert!(!required_check_done_mentions(
            "not_http_probe done",
            "http_probe"
        ));
    }

    // P2 · validate-on-submit ACCEPT: structure OK *and* every cited evidence id
    // exists in the ledger → accepted.
    #[tokio::test]
    async fn validate_on_submit_accepts_when_evidence_exists() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        // valid_scoping_args cites evidence id 1.
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink))
            .with_evidence_repo(Arc::new(MockLedger::existing([1].into_iter().collect())));

        let out = tool
            .execute(valid_scoping_args(), Path::new("/tmp"))
            .await
            .unwrap();
        assert_eq!(out["status"].as_str(), Some("accepted"));
        assert!(sink.read().await.is_some());
    }

    // P2 · validate-on-submit REJECT: structure OK but a cited evidence id is NOT
    // in the ledger → needs_fix NOW (not a misleading `accepted`), naming the
    // fabricated id; the deliverable is still stashed for the stage-close gate.
    #[tokio::test]
    async fn validate_on_submit_needs_fix_when_evidence_fabricated() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        // Empty ledger → cited id 1 is fabricated.
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink))
            .with_evidence_repo(Arc::new(MockLedger::existing(HashSet::new())));

        let out = tool
            .execute(valid_scoping_args(), Path::new("/tmp"))
            .await
            .unwrap();
        assert_eq!(out["status"].as_str(), Some("needs_fix"));
        let fabricated = out["fabricated_evidence_refs"]
            .as_array()
            .expect("fabricated list");
        assert_eq!(fabricated, &vec![serde_json::json!(1)]);
        assert!(sink.read().await.is_some());
    }

    // 乙 · a fabricated-ref needs_fix must NAME the operation's real evidence ids
    // (scoped by session) so the model re-cites real ids instead of placeholders.
    #[tokio::test]
    async fn fabricated_needs_fix_lists_available_real_ids() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        // Nothing the deliverable cited exists, but the operation DOES have real
        // ids 88, 86 recorded — they must be surfaced for re-citation.
        let ledger = MockLedger {
            existing: HashSet::new(),
            recent: vec![88, 86],
            kinds: HashMap::new(),
            facts: Vec::new(),
            source_queries: Vec::new(),
        };
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink))
            .with_evidence_repo(Arc::new(ledger))
            .with_session_id("pentest-chat-1");

        let out = tool
            .execute(valid_scoping_args(), Path::new("/tmp"))
            .await
            .unwrap();
        assert_eq!(out["status"].as_str(), Some("needs_fix"));
        let available = out["available_evidence_ids"]
            .as_array()
            .expect("available_evidence_ids present");
        assert_eq!(
            available,
            &vec![serde_json::json!(88), serde_json::json!(86)]
        );
        let reason = out["reasons"][0].as_str().unwrap();
        assert!(
            reason.contains("88") && reason.contains("86"),
            "reason names the real ids: {reason}"
        );
    }

    // 乙 · without a session_id the suggestion degrades gracefully (empty list +
    // optional-id wording), still rejecting the fabricated ref.
    #[tokio::test]
    async fn fabricated_needs_fix_without_session_degrades() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let ledger = MockLedger {
            existing: HashSet::new(),
            recent: vec![88, 86],
            kinds: HashMap::new(),
            facts: Vec::new(),
            source_queries: Vec::new(),
        };
        // No .with_session_id → available_real_ids() returns empty.
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink))
            .with_evidence_repo(Arc::new(ledger));

        let out = tool
            .execute(valid_scoping_args(), Path::new("/tmp"))
            .await
            .unwrap();
        assert_eq!(out["status"].as_str(), Some("needs_fix"));
        assert!(out["available_evidence_ids"]
            .as_array()
            .expect("available_evidence_ids present")
            .is_empty());
        let reason = out["reasons"][0].as_str().unwrap();
        assert!(
            reason.contains("Evidence ids are optional")
                && reason.contains("remove these id fields")
                && reason.contains("Never invent"),
            "degraded reason tells the model to omit fabricated ids: {reason}"
        );
    }

    // F1 · a structural/vacuous block (empty evidence) must ALSO surface the
    // operation's real evidence ids — not only the fabricated-ref branch — so an
    // agent that submitted empty because it could not find ids is handed them
    // right at the failure instead of looping to hunt for a non-existent field.
    #[tokio::test]
    async fn vacuous_needs_fix_lists_available_real_ids() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let ledger = MockLedger {
            existing: HashSet::new(),
            recent: vec![644, 646],
            kinds: HashMap::new(),
            facts: Vec::new(),
            source_queries: Vec::new(),
        };
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink))
            .with_evidence_repo(Arc::new(ledger))
            .with_session_id("pentest-chat-1");

        // Empty claims/refs → fails the structural gate (NOT the fabricated-ref
        // branch, which only fires when ids are actually cited).
        let args = json!({
            "stage_id": "scoping",
            "stage_run_id": "3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
            "claims": [],
            "evidence_refs": [],
            "findings": []
        });
        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("needs_fix"));
        let available = out["available_evidence_ids"]
            .as_array()
            .expect("available_evidence_ids present on a structural block");
        assert_eq!(available, &vec![json!(644), json!(646)]);
        // Real ids stay available as debug context, not as a reason that tells
        // the model to copy them into the deliverable.
        let reasons = out["reasons"].as_array().expect("reasons array");
        assert!(
            !reasons.iter().any(|r| {
                let s = r.as_str().unwrap_or("");
                s.contains("644") && s.contains("646")
            }),
            "structural reasons should not tell the model to copy ids: {reasons:?}"
        );
        assert!(sink.read().await.is_some());
    }

    // Without an evidence repo (None), the check is skipped and a structurally
    // valid deliverable is accepted (deferring fabrication detection to the
    // authoritative stage-close gate) — the pre-P2 behaviour stays intact.
    #[tokio::test]
    async fn no_evidence_repo_skips_check_and_accepts() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));
        let out = tool
            .execute(valid_scoping_args(), Path::new("/tmp"))
            .await
            .unwrap();
        assert_eq!(out["status"].as_str(), Some("accepted"));
    }

    // 正法 (2026-06-14) · the submit-time preview must credit an authoritative
    // `found` coverage cell from a REAL ledger evidence-fact. target_intel runs
    // coverage_complete with authoritative_found=true, so a `found` DNS cell is
    // valid only when a ledger fact (asset × technique × Found) exists. The bug:
    // the preview ran on a `None`/default GateContext (no facts), so it rejected
    // EVERY found cell as "never attempted" — trapping per-org recon sub-agents in
    // an endless resubmit loop even though the dig fact was already in the ledger
    // (confirmed: 5 (pingan.com × DNS × found) rows in the live audit_log).
    #[tokio::test]
    async fn target_intel_found_cell_credited_from_evidence_facts() {
        use golish_agent_kit::harness::EvidenceOutcome;
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::TargetIntel);

        // The ledger holds the real DNS fact for pingan.com (the dig already ran)
        // and recognises the cited evidence ids. Two ids so the deliverable clears
        // target_intel's min_invocations sum (dns_resolve + subdomain_enum_passive).
        let ledger = MockLedger {
            existing: [100, 101].into_iter().collect(),
            recent: vec![101, 100],
            kinds: HashMap::new(),
            facts: vec![EvidenceFact {
                asset: "pingan.com".into(),
                technique: "GOLISH-INTEL-DNS".into(),
                outcome: EvidenceOutcome::Found,
                evidence_id: 100,
            }],
            source_queries: vec![
                SourceQueryFact {
                    source: "dig".into(),
                    query: "dns_resolve".into(),
                    target: "pingan.com".into(),
                    technique: Some("GOLISH-INTEL-DNS".into()),
                    status: "found".into(),
                    evidence_ids: vec![100],
                },
                SourceQueryFact {
                    source: "test".into(),
                    query: "certificate_transparency".into(),
                    target: "pingan.com".into(),
                    technique: Some("GOLISH-INTEL-CT".into()),
                    status: "blocked".into(),
                    evidence_ids: vec![],
                },
                SourceQueryFact {
                    source: "test".into(),
                    query: "subdomain_enumeration".into(),
                    target: "pingan.com".into(),
                    technique: Some("GOLISH-INTEL-SUBDOMAIN".into()),
                    status: "blocked".into(),
                    evidence_ids: vec![],
                },
            ],
        };
        let org_id = uuid::Uuid::new_v4();
        let org_key = format!("organization:{org_id}");
        let org_src = Arc::new(RwLock::new(Some(org_id)));
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink))
            .with_evidence_repo(Arc::new(ledger))
            .with_session_id("pentest-chat-1")
            .with_org_id_source(org_src);

        // DNS found (now backed by a real fact) + the other 5 expected techniques
        // marked blocked (terminal, no fact needed) = full coverage for pingan.com.
        let blocked = |t: &str| {
            json!({ "asset": "pingan.com", "technique": t, "status": "blocked",
                    "note": "no provider/tool registered for this technique" })
        };
        let args = json!({
            "stage_id": "target_intel",
            "stage_run_id": "3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
            "claims": [{
                "kind": "dns_a_record", "subject": "pingan.com",
                "summary": "pingan.com A 183.62.123.62", "evidence_ids": [100],
                "technique": "GOLISH-INTEL-DNS"
            }],
            "evidence_refs": [100, 101],
            "findings": [],
            "coverage": [
                { "asset": "pingan.com", "technique": "GOLISH-INTEL-DNS",
                  "status": "found", "evidence_refs": [100] },
                blocked("GOLISH-INTEL-WHOIS"), blocked("GOLISH-INTEL-ASN"),
                blocked("GOLISH-INTEL-CT"), blocked("GOLISH-INTEL-SUBDOMAIN"),
                blocked("GOLISH-INTEL-OSINT"),
                { "asset": org_key, "technique": "GOLISH-INTEL-WHOIS", "status": "blocked",
                  "note": "no registration source configured" },
                { "asset": org_key, "technique": "GOLISH-INTEL-ASN", "status": "blocked",
                  "note": "no ASN source configured" },
                { "asset": org_key, "technique": "GOLISH-INTEL-OSINT", "status": "blocked",
                  "note": "no OSINT source configured" }
            ],
            "skipped_checks": [],
            "required_checks_done": ["dns_resolve", "subdomain_enum_passive"]
        });

        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(
            out["status"].as_str(),
            Some("accepted"),
            "DNS found must be credited from the ledger fact, not rejected as never-attempted: {out:?}"
        );
    }

    // 2026-06-16 · companion to the ledger-fact test above: the submit preview
    // must ALSO credit found cells from the ORG-keyed DB business-table truth
    // (coverage_truth → organizations.asns/.certificates/.intel = ASN/CT/OSINT).
    // Those techniques have NO CLI tool, so they never appear in the session-keyed
    // command-path `evidence_facts`; before this fix the preview ran without the
    // db-truth half AND without an authoritative asset axis, so it marked every
    // ASN/CT/OSINT cell "never attempted" and dead-looped per-org recon sub-agents
    // even after enrich landed the data (live moresec.cn run: certificates=3
    // landed in organizations.certificates yet CT stayed "never attempted").
    struct DbTruthMock {
        existing: HashSet<i64>,
        db_facts: Vec<EvidenceFact>,
        assets: Vec<String>,
        source_queries: Vec<SourceQueryFact>,
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for DbTruthMock {
        async fn existing_evidence_ids(&self, ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(ids
                .iter()
                .copied()
                .filter(|i| self.existing.contains(i))
                .collect())
        }
        // No command-path ledger facts on purpose (evidence_facts defaults empty):
        // this proves the db-truth half alone satisfies ASN/CT/OSINT.
        async fn db_truth_facts(
            &self,
            _org: Option<uuid::Uuid>,
            _assets: &[String],
        ) -> Vec<EvidenceFact> {
            self.db_facts.clone()
        }
        async fn in_scope_assets(&self, _org: Option<uuid::Uuid>) -> Vec<String> {
            self.assets.clone()
        }
        async fn source_query_facts(
            &self,
            _org_id: uuid::Uuid,
            _run_id: &str,
        ) -> Vec<SourceQueryFact> {
            self.source_queries.clone()
        }
    }

    #[tokio::test]
    async fn target_intel_found_cells_credited_from_db_truth_facts() {
        use golish_agent_kit::harness::EvidenceOutcome;
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::TargetIntel);

        let found = |t: &str| EvidenceFact {
            asset: "moresec.cn".into(),
            technique: t.into(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 0, // sentinel: business-table truth, not a ledger row
        };
        let org_id = uuid::Uuid::new_v4();
        let org_key = format!("organization:{org_id}");
        let org_found = |t: &str| EvidenceFact {
            asset: org_key.clone(),
            technique: t.into(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 0,
        };
        let repo = DbTruthMock {
            existing: [100, 101].into_iter().collect(),
            db_facts: vec![
                found("GOLISH-INTEL-DNS"),
                found("GOLISH-INTEL-WHOIS"),
                found("GOLISH-INTEL-ASN"),
                found("GOLISH-INTEL-CT"),
                found("GOLISH-INTEL-SUBDOMAIN"),
                found("GOLISH-INTEL-OSINT"),
                org_found("GOLISH-INTEL-WHOIS"),
                org_found("GOLISH-INTEL-ASN"),
                org_found("GOLISH-INTEL-OSINT"),
            ],
            assets: vec!["moresec.cn".into()],
            source_queries: vec![
                SourceQueryFact {
                    source: "provider_status".into(),
                    query: "recon_map_assets".into(),
                    target: "moresec.cn".into(),
                    technique: None,
                    status: "found".into(),
                    evidence_ids: vec![100],
                },
                SourceQueryFact {
                    source: "rdap".into(),
                    query: "lookup_whois".into(),
                    target: "moresec.cn".into(),
                    technique: Some("GOLISH-INTEL-WHOIS".into()),
                    status: "found".into(),
                    evidence_ids: vec![101],
                },
            ],
        };
        let org_src = Arc::new(RwLock::new(Some(org_id)));
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink))
            .with_evidence_repo(Arc::new(repo))
            .with_session_id("pentest-chat-1")
            .with_org_id_source(org_src);

        // The model submits NO coverage cells (methodology: "leave found cells out
        // — the DB supplies them"); just a backing claim + cited ids to clear the
        // structural checks. All 6 techniques must come from the db-truth half.
        let args = json!({
            "stage_id": "target_intel",
            "stage_run_id": "3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
            "claims": [{
                "kind": "dns_a_record", "subject": "moresec.cn",
                "summary": "moresec.cn A 115.28.135.55", "evidence_ids": [100],
                "technique": "GOLISH-INTEL-DNS"
            }],
            "evidence_refs": [100, 101],
            "findings": [],
            "coverage": [],
            "skipped_checks": [],
            "required_checks_done": ["dns_resolve", "subdomain_enum_passive"]
        });

        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(
            out["status"].as_str(),
            Some("accepted"),
            "ASN/CT/OSINT must be credited from DB business-table truth, not rejected as never-attempted: {out:?}"
        );
    }

    #[tokio::test]
    async fn target_intel_organization_only_preview_matches_org_gate_context() {
        use golish_agent_kit::harness::EvidenceOutcome;

        let org_id = uuid::Uuid::from_u128(7);
        let org_key = format!("organization:{org_id}");
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::TargetIntel);

        let found = |technique: &str| EvidenceFact {
            asset: org_key.clone(),
            technique: technique.to_string(),
            outcome: EvidenceOutcome::Found,
            evidence_id: 0,
        };
        let repo = DbTruthMock {
            existing: [100, 101].into_iter().collect(),
            db_facts: vec![
                found("GOLISH-INTEL-WHOIS"),
                found("GOLISH-INTEL-ASN"),
                found("GOLISH-INTEL-OSINT"),
            ],
            assets: Vec::new(),
            source_queries: vec![
                SourceQueryFact {
                    source: "provider_status".into(),
                    query: "recon_map_assets".into(),
                    target: String::new(),
                    technique: None,
                    status: "found".into(),
                    evidence_ids: vec![100],
                },
                SourceQueryFact {
                    source: "rdap".into(),
                    query: "lookup_whois".into(),
                    target: String::new(),
                    technique: Some("GOLISH-INTEL-WHOIS".into()),
                    status: "found".into(),
                    evidence_ids: vec![101],
                },
            ],
        };
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(repo))
            .with_session_id("pentest-chat-org-only")
            .with_org_id_source(Arc::new(RwLock::new(Some(org_id))));

        let context = tool
            .gate_context(StageKind::TargetIntel, true)
            .await
            .expect("organization-only Target Intel preview context");

        assert_eq!(context.in_scope_assets, Some(vec![org_key.clone()]));
        assert_eq!(
            context
                .asset_types
                .as_ref()
                .and_then(|types| types.get(&org_key))
                .map(String::as_str),
            Some("organization")
        );
        let not_applicable = context
            .not_applicable_coverage
            .as_ref()
            .expect("organization-only context must carry deterministic N/A cells");
        for technique in [
            "GOLISH-INTEL-DNS",
            "GOLISH-INTEL-CT",
            "GOLISH-INTEL-SUBDOMAIN",
        ] {
            assert!(
                not_applicable.contains(&(org_key.clone(), technique.to_string())),
                "organization context must mark {technique} not_applicable"
            );
        }
        for technique in [
            "GOLISH-INTEL-WHOIS",
            "GOLISH-INTEL-ASN",
            "GOLISH-INTEL-OSINT",
        ] {
            assert!(
                context.evidence_facts.as_ref().is_some_and(|facts| facts
                    .iter()
                    .any(|fact| fact.asset == org_key && fact.technique == technique)),
                "organization context must query DB truth for {technique}"
            );
        }

        let out = tool
            .execute(
                json!({
                    "stage_id": "target_intel",
                    "claims": [{
                        "kind": "organization_intel",
                        "subject": org_key,
                        "summary": "organization context collected",
                        "evidence_ids": [100],
                        "technique": "GOLISH-INTEL-OSINT"
                    }],
                    "evidence_refs": [100, 101],
                    "coverage": []
                }),
                Path::new("/tmp"),
            )
            .await
            .expect("organization-only slim submit preview");
        assert_eq!(
            out["status"].as_str(),
            Some("accepted"),
            "organization-only coverage=[] must preview the same way as the final org gate: {out:?}"
        );
    }

    #[tokio::test]
    async fn enumeration_submit_preview_requires_non_empty_session() {
        use golish_agent_kit::harness::EvidenceOutcome;

        let (stage, sink) = handles();
        let origin = "https://app.example.com:443".to_string();
        let repo = DbTruthMock {
            existing: HashSet::new(),
            db_facts: vec![EvidenceFact {
                asset: origin.clone(),
                technique: "GOLISH-ENUM-DIR".to_string(),
                outcome: EvidenceOutcome::Found,
                evidence_id: 0,
            }],
            assets: vec![origin],
            source_queries: Vec::new(),
        };
        let org_src = Arc::new(RwLock::new(Some(uuid::Uuid::new_v4())));
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(repo))
            .with_org_id_source(org_src);

        let error = tool
            .gate_context(StageKind::Enumeration, true)
            .await
            .expect_err("Enumeration preview must not run without a current session");

        assert!(error.contains("non-empty run/session id"), "{error}");
    }

    #[tokio::test]
    async fn vuln_submit_preview_requires_non_empty_evidence_session() {
        let (stage, sink) = handles();
        let tool = SubmitStageDeliverableTool::new(stage, sink);

        let error = tool
            .gate_context(StageKind::VulnTriage, true)
            .await
            .expect_err("Vuln preview must not run without the wrapper evidence session");

        assert!(
            error.contains(
                "vuln_triage submit preview requires the active non-empty run/session id"
            ),
            "{error}"
        );
    }

    struct StaleEnumerationOutcomeMock;

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for StaleEnumerationOutcomeMock {
        async fn existing_evidence_ids(&self, _ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(HashSet::new())
        }

        async fn in_scope_assets(&self, _org: Option<uuid::Uuid>) -> Vec<String> {
            vec!["https://app.example.com:443".to_string()]
        }

        async fn technique_outcome_facts(
            &self,
            _org_id: uuid::Uuid,
            _run_id: &str,
        ) -> Vec<TechniqueOutcomeFact> {
            vec![TechniqueOutcomeFact::new(
                "https://app.example.com:443".to_string(),
                "GOLISH-ENUM-DIR".to_string(),
                "found".to_string(),
                91,
                Some("route_probe_paths".to_string()),
            )]
        }
    }

    #[tokio::test]
    async fn enumeration_submit_preview_requires_freshness_cutoff() {
        let (stage, sink) = handles();
        let org_src = Arc::new(RwLock::new(Some(uuid::Uuid::new_v4())));
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(StaleEnumerationOutcomeMock))
            .with_session_id("reused-session")
            .with_org_id_source(org_src);

        let error = tool
            .gate_context(StageKind::Enumeration, true)
            .await
            .expect_err("Enumeration preview must not run without a stage cutoff");

        assert!(
            error.contains("stage_started_at freshness cutoff"),
            "{error}"
        );
    }

    struct EnumerationCoverageBoundaryMock {
        operation_id: uuid::Uuid,
        snapshot_present: bool,
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for EnumerationCoverageBoundaryMock {
        async fn existing_evidence_ids(&self, _ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(HashSet::new())
        }

        async fn operation_stage_started_at(
            &self,
            operation_id: uuid::Uuid,
        ) -> Option<(StageKind, chrono::DateTime<chrono::Utc>)> {
            assert_eq!(operation_id, self.operation_id);
            Some((StageKind::Enumeration, chrono::Utc::now()))
        }

        async fn stage_asset_coverage(
            &self,
            organization_id: uuid::Uuid,
            _stage: StageKind,
            session_id: Option<&str>,
            _stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
            _current_wave_target_ids: Option<Vec<uuid::Uuid>>,
            _current_wave_asset_values: Option<Vec<String>>,
        ) -> Result<Option<serde_json::Value>> {
            Ok(self.snapshot_present.then(|| {
                json!({
                    "stage": "enumeration",
                    "organization_id": organization_id,
                    "session_id": session_id,
                    "assets": []
                })
            }))
        }
    }

    #[tokio::test]
    async fn enumeration_submit_preserves_authoritative_empty_coverage_axis() {
        let (stage, sink) = handles();
        let operation_id = uuid::Uuid::new_v4();
        let org_src = Arc::new(RwLock::new(Some(uuid::Uuid::new_v4())));
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(EnumerationCoverageBoundaryMock {
                operation_id,
                snapshot_present: true,
            }))
            .with_session_id("authoritative-zero-run")
            .with_org_id_source(org_src)
            .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))));

        let ctx = tool
            .gate_context(StageKind::Enumeration, true)
            .await
            .unwrap();

        assert_eq!(ctx.in_scope_assets, Some(Vec::new()));
    }

    #[tokio::test]
    async fn enumeration_submit_preview_requires_bound_organization() {
        let (stage, sink) = handles();
        let operation_id = uuid::Uuid::new_v4();
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(EnumerationCoverageBoundaryMock {
                operation_id,
                snapshot_present: true,
            }))
            .with_session_id("missing-org-run")
            .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))));

        let error = tool
            .gate_context(StageKind::Enumeration, true)
            .await
            .expect_err("Enumeration preview must require its bound organization");

        assert!(error.contains("bound organization"), "{error}");
    }

    #[tokio::test]
    async fn enumeration_submit_preview_rejects_absent_snapshot() {
        let (stage, sink) = handles();
        let operation_id = uuid::Uuid::new_v4();
        let organization_id = uuid::Uuid::new_v4();
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(EnumerationCoverageBoundaryMock {
                operation_id,
                snapshot_present: false,
            }))
            .with_session_id("missing-snapshot-run")
            .with_org_id_source(Arc::new(RwLock::new(Some(organization_id))))
            .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))));

        let error = tool
            .gate_context(StageKind::Enumeration, true)
            .await
            .expect_err("Enumeration preview must distinguish no snapshot from an empty axis");

        assert!(error.contains("snapshot is unavailable"), "{error}");
    }

    // T3 (2026-06-23 · 设计 submit-preview-authoritative-context) · the submit
    // preview gate context feeds host-aware `asset_types` + dynamic
    // `expected_techniques` (matching the stage-close口径) ONLY when the
    // authoritative gray-switch is on; off ⇒ prior behaviour (axis + facts only).
    struct TypedAxisMock {
        assets: Vec<String>,
        typed: Vec<(String, String)>,
        target_types: Vec<String>,
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for TypedAxisMock {
        async fn existing_evidence_ids(&self, ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(ids.iter().copied().collect())
        }
        async fn in_scope_assets(&self, _org: Option<uuid::Uuid>) -> Vec<String> {
            self.assets.clone()
        }
        async fn in_scope_typed_assets(&self, _org: Option<uuid::Uuid>) -> Vec<(String, String)> {
            self.typed.clone()
        }
        async fn in_scope_target_types(&self, _org: Option<uuid::Uuid>) -> Vec<String> {
            self.target_types.clone()
        }
    }

    #[tokio::test]
    async fn submit_preview_authoritative_flag_gates_asset_types_and_expected_techniques() {
        let (stage, sink) = handles();
        let repo = TypedAxisMock {
            assets: vec!["moresec.cn".into()],
            typed: vec![("moresec.cn".into(), "domain".into())],
            target_types: vec!["domain".into()],
        };
        let org_src = Arc::new(RwLock::new(Some(uuid::Uuid::new_v4())));
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(repo))
            .with_session_id("pentest-chat-1")
            .with_org_id_source(org_src);

        // flag OFF = prior behaviour: asset axis still fed, but asset_types /
        // expected_techniques stay None (preview keeps value-inference + static spec).
        let off = tool
            .gate_context(StageKind::TargetIntel, false)
            .await
            .unwrap();
        assert!(
            off.in_scope_assets.is_some(),
            "asset axis still fed when off"
        );
        assert!(
            off.asset_types.is_none(),
            "asset_types omitted when flag off"
        );
        assert!(
            off.expected_techniques.is_none(),
            "expected_techniques omitted when flag off"
        );

        // flag ON = authoritative口径: both populated (matches stage-close gate).
        let on = tool
            .gate_context(StageKind::TargetIntel, true)
            .await
            .unwrap();
        assert!(on.asset_types.is_some(), "asset_types fed when flag on");
        assert!(
            on.expected_techniques.is_some(),
            "dynamic expected_techniques fed when flag on"
        );
    }

    // 设计 2026-07-01 §5.3 · the submit preview must feed EAS/httpx-proven IP web
    // roots into `web_capable_assets` for enumeration (spec opts into
    // enum_ip_web_coverage), so a web-capable IP is held to the four content axes
    // by the preview instead of previewing PASS then blocking at stage-close.
    struct WebCapableMock {
        operation_id: uuid::Uuid,
        web_capable: Vec<String>,
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for WebCapableMock {
        async fn existing_evidence_ids(&self, ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(ids.iter().copied().collect())
        }
        async fn in_scope_assets(&self, _org: Option<uuid::Uuid>) -> Vec<String> {
            vec!["1.2.3.4".to_string()]
        }
        async fn operation_stage_started_at(
            &self,
            operation_id: uuid::Uuid,
        ) -> Option<(StageKind, chrono::DateTime<chrono::Utc>)> {
            assert_eq!(operation_id, self.operation_id);
            Some((StageKind::Enumeration, chrono::Utc::now()))
        }
        async fn stage_asset_coverage(
            &self,
            organization_id: uuid::Uuid,
            _stage: StageKind,
            session_id: Option<&str>,
            _stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
            _current_wave_target_ids: Option<Vec<uuid::Uuid>>,
            _current_wave_asset_values: Option<Vec<String>>,
        ) -> Result<Option<Value>> {
            Ok(Some(json!({
                "stage": "enumeration",
                "organization_id": organization_id,
                "session_id": session_id,
                "assets": [{
                    "value": "https://1.2.3.4:443",
                    "target_type": "url",
                    "exact_web_origin": true
                }]
            })))
        }
        async fn enumeration_web_capable_assets(&self, _org: Option<uuid::Uuid>) -> Vec<String> {
            self.web_capable.clone()
        }
    }

    #[tokio::test]
    async fn submit_preview_feeds_enumeration_web_capable_ip_roots() {
        let (stage, sink) = handles();
        let operation_id = uuid::Uuid::new_v4();
        let repo = WebCapableMock {
            operation_id,
            web_capable: vec!["1.2.3.4".to_string()],
        };
        let org_src = Arc::new(RwLock::new(Some(uuid::Uuid::new_v4())));
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(repo))
            .with_session_id("pentest-chat-1")
            .with_org_id_source(org_src)
            .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))));

        // Enumeration spec opts into enum_ip_web_coverage ⇒ the proven IP web root
        // is fed into web_capable_assets (matches org_gate / stage-close口径).
        let en = tool
            .gate_context(StageKind::Enumeration, true)
            .await
            .unwrap();
        let web = en
            .web_capable_assets
            .expect("web_capable_assets fed for enumeration");
        assert!(web.contains("1.2.3.4"));

        // Non-enumeration stage ⇒ never injected (stays None = prior behaviour).
        let ti = tool
            .gate_context(StageKind::TargetIntel, true)
            .await
            .unwrap();
        assert!(
            ti.web_capable_assets.is_none(),
            "web_capable only injected for enumeration"
        );
    }

    struct EnumerationOriginAxisMock {
        operation_id: uuid::Uuid,
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for EnumerationOriginAxisMock {
        async fn existing_evidence_ids(&self, ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(ids.iter().copied().collect())
        }

        async fn in_scope_assets(&self, _org: Option<uuid::Uuid>) -> Vec<String> {
            vec!["app.example.com".to_string()]
        }

        async fn operation_stage_started_at(
            &self,
            operation_id: uuid::Uuid,
        ) -> Option<(StageKind, chrono::DateTime<chrono::Utc>)> {
            assert_eq!(operation_id, self.operation_id);
            Some((StageKind::Enumeration, chrono::Utc::now()))
        }

        async fn stage_asset_coverage(
            &self,
            organization_id: uuid::Uuid,
            _stage: StageKind,
            session_id: Option<&str>,
            _stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
            _current_wave_target_ids: Option<Vec<uuid::Uuid>>,
            _current_wave_asset_values: Option<Vec<String>>,
        ) -> Result<Option<Value>> {
            Ok(Some(json!({
                "stage": "enumeration",
                "organization_id": organization_id,
                "session_id": session_id,
                "assets": [
                    {"value": "http://app.example.com:80", "target_type": "url", "exact_web_origin": true},
                    {"value": "https://app.example.com:443", "target_type": "url", "exact_web_origin": true},
                    {"value": "https://203.0.113.10:443", "target_type": "url", "exact_web_origin": true}
                ]
            })))
        }
    }

    type CapturedWaveCoverageCall = (
        Option<chrono::DateTime<chrono::Utc>>,
        Option<Vec<uuid::Uuid>>,
        Option<Vec<String>>,
    );

    struct CurrentWavePreviewMock {
        operation_id: uuid::Uuid,
        organization_id: uuid::Uuid,
        operation_started_at: chrono::DateTime<chrono::Utc>,
        wave_started_at: chrono::DateTime<chrono::Utc>,
        target_id: uuid::Uuid,
        coverage_call: Arc<std::sync::Mutex<Option<CapturedWaveCoverageCall>>>,
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for CurrentWavePreviewMock {
        async fn existing_evidence_ids(&self, _ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(HashSet::new())
        }

        async fn operation_stage_started_at(
            &self,
            operation_id: uuid::Uuid,
        ) -> Option<(StageKind, chrono::DateTime<chrono::Utc>)> {
            assert_eq!(operation_id, self.operation_id);
            Some((StageKind::Enumeration, self.operation_started_at))
        }

        async fn stage_asset_wave_current_running(
            &self,
            operation_id: uuid::Uuid,
            organization_id: uuid::Uuid,
            stage: StageKind,
        ) -> Result<Option<StageAssetWaveView>> {
            assert_eq!(operation_id, self.operation_id);
            assert_eq!(organization_id, self.organization_id);
            assert_eq!(stage, StageKind::Enumeration);
            Ok(Some(StageAssetWaveView {
                id: uuid::Uuid::from_u128(100),
                operation_id,
                organization_id,
                stage_kind: stage.as_str().to_string(),
                wave_index: 1,
                started_at: self.wave_started_at,
                parent_wave_id: Some(uuid::Uuid::from_u128(99)),
                asset_hash: "wave-hash".to_string(),
                target_ids: vec![self.target_id],
                asset_values: vec!["wave.example.com".to_string()],
            }))
        }

        async fn stage_asset_coverage(
            &self,
            organization_id: uuid::Uuid,
            stage: StageKind,
            session_id: Option<&str>,
            stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
            current_wave_target_ids: Option<Vec<uuid::Uuid>>,
            current_wave_asset_values: Option<Vec<String>>,
        ) -> Result<Option<Value>> {
            assert_eq!(organization_id, self.organization_id);
            assert_eq!(stage, StageKind::Enumeration);
            *self.coverage_call.lock().unwrap() = Some((
                stage_started_at,
                current_wave_target_ids,
                current_wave_asset_values,
            ));
            Ok(Some(json!({
                "stage": "enumeration",
                "organization_id": organization_id,
                "session_id": session_id,
                "assets": [{
                    "value": "https://wave.example.com:443",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": []
                }]
            })))
        }

        async fn in_scope_assets_created_before(
            &self,
            _org_id: Option<uuid::Uuid>,
            cutoff: chrono::DateTime<chrono::Utc>,
        ) -> Vec<String> {
            assert_eq!(cutoff, self.wave_started_at);
            vec![
                "wrong-cutoff-only.example.com".to_string(),
                "wave.example.com".to_string(),
            ]
        }

        async fn in_scope_typed_assets(
            &self,
            _org_id: Option<uuid::Uuid>,
        ) -> Vec<(String, String)> {
            vec![
                (
                    "wrong-cutoff-only.example.com".to_string(),
                    "domain".to_string(),
                ),
                ("wave.example.com".to_string(), "domain".to_string()),
            ]
        }

        async fn in_scope_target_types(&self, _org_id: Option<uuid::Uuid>) -> Vec<String> {
            vec!["domain".to_string()]
        }
    }

    #[tokio::test]
    async fn enumeration_submit_preview_forwards_same_wave_cutoff_ids_and_values() {
        let (stage, sink) = handles();
        let operation_id = uuid::Uuid::new_v4();
        let organization_id = uuid::Uuid::new_v4();
        let operation_started_at = chrono::Utc::now() - chrono::Duration::minutes(10);
        let wave_started_at = chrono::Utc::now() - chrono::Duration::minutes(2);
        let target_id = uuid::Uuid::new_v4();
        let coverage_call = Arc::new(std::sync::Mutex::new(None));
        let repo = CurrentWavePreviewMock {
            operation_id,
            organization_id,
            operation_started_at,
            wave_started_at,
            target_id,
            coverage_call: Arc::clone(&coverage_call),
        };
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(repo))
            .with_session_id("wave-preview-run")
            .with_org_id_source(Arc::new(RwLock::new(Some(organization_id))))
            .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))));

        let ctx = tool
            .gate_context(StageKind::Enumeration, true)
            .await
            .unwrap();

        assert_eq!(
            ctx.in_scope_assets,
            Some(vec!["https://wave.example.com:443".to_string()]),
            "Enumeration submit preview must use the exact-origin axis produced from the durable wave"
        );
        let captured = coverage_call
            .lock()
            .unwrap()
            .clone()
            .expect("Enumeration coverage projection must be called");
        assert_eq!(captured.0, Some(wave_started_at));
        assert_eq!(captured.1, Some(vec![target_id]));
        assert_eq!(captured.2, Some(vec!["wave.example.com".to_string()]));
    }

    struct EnumerationBlockedPreviewMock {
        operation_id: uuid::Uuid,
        outcome_source: &'static str,
        technique: &'static str,
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for EnumerationBlockedPreviewMock {
        async fn existing_evidence_ids(&self, ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(ids.iter().copied().collect())
        }

        async fn operation_stage_started_at(
            &self,
            operation_id: uuid::Uuid,
        ) -> Option<(StageKind, chrono::DateTime<chrono::Utc>)> {
            assert_eq!(operation_id, self.operation_id);
            Some((StageKind::Enumeration, chrono::Utc::now()))
        }

        async fn stage_asset_coverage(
            &self,
            organization_id: uuid::Uuid,
            _stage: StageKind,
            session_id: Option<&str>,
            _stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
            _current_wave_target_ids: Option<Vec<uuid::Uuid>>,
            _current_wave_asset_values: Option<Vec<String>>,
        ) -> Result<Option<Value>> {
            Ok(Some(json!({
                "stage": "enumeration",
                "organization_id": organization_id,
                "session_id": session_id,
                "assets": [{
                    "value": "https://203.0.113.10:443",
                    "target_type": "url",
                    "exact_web_origin": true,
                    "coverage": []
                }]
            })))
        }

        async fn evidence_facts(&self, _session_id: &str) -> Vec<EvidenceFact> {
            vec![EvidenceFact {
                asset: "https://203.0.113.10:443".to_string(),
                technique: self.technique.to_string(),
                outcome: golish_agent_kit::harness::EvidenceOutcome::Blocked,
                evidence_id: 73,
            }]
        }

        async fn technique_outcome_facts_fresh(
            &self,
            _org_id: uuid::Uuid,
            _run_id: &str,
            _since: Option<chrono::DateTime<chrono::Utc>>,
        ) -> Vec<TechniqueOutcomeFact> {
            vec![TechniqueOutcomeFact::new(
                "https://203.0.113.10:443",
                self.technique,
                "blocked",
                73,
                Some(self.outcome_source.to_string()),
            )]
        }
    }

    #[tokio::test]
    async fn enumeration_submit_preview_requires_trusted_blocked_source_and_axis() {
        for (source, technique, expected) in [
            ("enum_preflight_web_origins", "GOLISH-ENUM-JS", true),
            ("enum_preflight_web_origins", "GOLISH-ENUM-DIR", true),
            ("enum_preflight_web_origins", "GOLISH-ENUM-PARAM", true),
            ("enum_preflight_web_origins", "GOLISH-ENUM-JSAPI", true),
            ("route_probe_paths", "GOLISH-ENUM-DIR", true),
            ("route_probe_paths", "GOLISH-ENUM-JS", false),
            ("browser_collect_js_api", "GOLISH-ENUM-JS", true),
            ("browser_collect_js_api", "GOLISH-ENUM-JSAPI", true),
            ("browser_collect_js_api", "GOLISH-ENUM-PARAM", true),
            ("browser_collect_js_api", "GOLISH-ENUM-DIR", false),
            ("untrusted_probe", "GOLISH-ENUM-DIR", false),
        ] {
            let (stage, sink) = handles();
            let operation_id = uuid::Uuid::new_v4();
            let repo = EnumerationBlockedPreviewMock {
                operation_id,
                outcome_source: source,
                technique,
            };
            let tool = SubmitStageDeliverableTool::new(stage, sink)
                .with_evidence_repo(Arc::new(repo))
                .with_session_id("blocked-preview-run")
                .with_org_id_source(Arc::new(RwLock::new(Some(uuid::Uuid::new_v4()))))
                .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))));

            let context = tool
                .gate_context(StageKind::Enumeration, true)
                .await
                .unwrap();
            let projected = context.evidence_facts.unwrap_or_default();
            assert_eq!(
                projected.iter().any(|fact| {
                    fact.technique == technique
                        && fact.outcome == golish_agent_kit::harness::EvidenceOutcome::Blocked
                }),
                expected,
                "submit preview source/axis trust mismatch for {source}/{technique}"
            );
        }
    }

    struct InvalidWavePreviewMock {
        operation_id: uuid::Uuid,
        organization_id: uuid::Uuid,
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for InvalidWavePreviewMock {
        async fn existing_evidence_ids(&self, _ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(HashSet::new())
        }

        async fn operation_stage_started_at(
            &self,
            operation_id: uuid::Uuid,
        ) -> Option<(StageKind, chrono::DateTime<chrono::Utc>)> {
            assert_eq!(operation_id, self.operation_id);
            Some((StageKind::Enumeration, chrono::Utc::now()))
        }

        async fn stage_asset_wave_current_running(
            &self,
            operation_id: uuid::Uuid,
            organization_id: uuid::Uuid,
            stage: StageKind,
        ) -> Result<Option<StageAssetWaveView>> {
            assert_eq!(operation_id, self.operation_id);
            assert_eq!(organization_id, self.organization_id);
            Ok(Some(StageAssetWaveView {
                id: uuid::Uuid::from_u128(200),
                operation_id,
                organization_id,
                stage_kind: stage.as_str().to_string(),
                wave_index: 0,
                started_at: chrono::Utc::now(),
                parent_wave_id: None,
                asset_hash: "empty".to_string(),
                target_ids: Vec::new(),
                asset_values: Vec::new(),
            }))
        }
    }

    #[tokio::test]
    async fn submit_preview_returns_needs_fix_for_present_empty_wave() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Enumeration);
        let operation_id = uuid::Uuid::new_v4();
        let organization_id = uuid::Uuid::new_v4();
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(InvalidWavePreviewMock {
                operation_id,
                organization_id,
            }))
            .with_session_id("empty-wave-run")
            .with_org_id_source(Arc::new(RwLock::new(Some(organization_id))))
            .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))));

        let out = tool
            .execute(
                json!({
                    "stage_id": "enumeration",
                    "claims": [],
                    "coverage": []
                }),
                Path::new("/tmp"),
            )
            .await
            .unwrap();

        assert_eq!(out["status"], "needs_fix");
        assert!(out["reasons"][0]
            .as_str()
            .is_some_and(|reason| reason.contains("has no items")));
    }

    struct SnapshotErrorPreviewMock {
        operation_id: uuid::Uuid,
        organization_id: uuid::Uuid,
        target_id: uuid::Uuid,
    }

    #[async_trait::async_trait]
    impl EvidenceLedgerQuery for SnapshotErrorPreviewMock {
        async fn existing_evidence_ids(&self, _ids: &[i64]) -> Result<HashSet<i64>> {
            Ok(HashSet::new())
        }

        async fn operation_stage_started_at(
            &self,
            operation_id: uuid::Uuid,
        ) -> Option<(StageKind, chrono::DateTime<chrono::Utc>)> {
            assert_eq!(operation_id, self.operation_id);
            Some((StageKind::Enumeration, chrono::Utc::now()))
        }

        async fn stage_asset_wave_current_running(
            &self,
            operation_id: uuid::Uuid,
            organization_id: uuid::Uuid,
            stage: StageKind,
        ) -> Result<Option<StageAssetWaveView>> {
            assert_eq!(operation_id, self.operation_id);
            assert_eq!(organization_id, self.organization_id);
            Ok(Some(StageAssetWaveView {
                id: uuid::Uuid::from_u128(201),
                operation_id,
                organization_id,
                stage_kind: stage.as_str().to_string(),
                wave_index: 0,
                started_at: chrono::Utc::now(),
                parent_wave_id: None,
                asset_hash: "snapshot-error".to_string(),
                target_ids: vec![self.target_id],
                asset_values: vec!["moved.example.com".to_string()],
            }))
        }

        async fn stage_asset_coverage(
            &self,
            _organization_id: uuid::Uuid,
            _stage: StageKind,
            _session_id: Option<&str>,
            _stage_started_at: Option<chrono::DateTime<chrono::Utc>>,
            _current_wave_target_ids: Option<Vec<uuid::Uuid>>,
            _current_wave_asset_values: Option<Vec<String>>,
        ) -> Result<Option<Value>> {
            Err(anyhow::anyhow!(
                "current wave target was deleted, moved, or left scope"
            ))
        }
    }

    #[tokio::test]
    async fn enumeration_submit_preview_surfaces_running_wave_snapshot_error() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Enumeration);
        let operation_id = uuid::Uuid::new_v4();
        let organization_id = uuid::Uuid::new_v4();
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(SnapshotErrorPreviewMock {
                operation_id,
                organization_id,
                target_id: uuid::Uuid::new_v4(),
            }))
            .with_session_id("snapshot-error-run")
            .with_org_id_source(Arc::new(RwLock::new(Some(organization_id))))
            .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))));

        let out = tool
            .execute(
                json!({
                    "stage_id": "enumeration",
                    "claims": [],
                    "coverage": []
                }),
                Path::new("/tmp"),
            )
            .await
            .unwrap();

        assert_eq!(out["status"], "needs_fix");
        assert!(out["reasons"][0]
            .as_str()
            .is_some_and(|reason| reason.contains("exact-origin coverage snapshot failed")));
        assert!(out["reasons"][0]
            .as_str()
            .is_some_and(|reason| reason.contains("deleted, moved, or left scope")));
    }

    #[tokio::test]
    async fn submit_preview_uses_exact_enumeration_origin_axis() {
        let (stage, sink) = handles();
        let operation_id = uuid::Uuid::new_v4();
        let org_src = Arc::new(RwLock::new(Some(uuid::Uuid::new_v4())));
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(EnumerationOriginAxisMock { operation_id }))
            .with_session_id("pentest-chat-1")
            .with_org_id_source(org_src)
            .with_operation_id_source(Arc::new(RwLock::new(Some(operation_id))));

        let ctx = tool
            .gate_context(StageKind::Enumeration, true)
            .await
            .unwrap();

        assert_eq!(
            ctx.in_scope_assets.as_ref().unwrap(),
            &vec![
                "http://app.example.com:80".to_string(),
                "https://app.example.com:443".to_string(),
                "https://203.0.113.10:443".to_string(),
            ]
        );
        let types = ctx.asset_types.as_ref().unwrap();
        assert_eq!(
            types.get("http://app.example.com:80"),
            Some(&"url".to_string())
        );
        assert_eq!(
            types.get("https://app.example.com:443"),
            Some(&"url".to_string())
        );
        assert_eq!(
            types.get("https://203.0.113.10:443"),
            Some(&"url".to_string())
        );

        let deliverable = StageDeliverable {
            stage_id: StageKind::Enumeration.as_str().to_string(),
            stage_run_id: uuid::Uuid::new_v4(),
            claims: Vec::new(),
            evidence_refs: Vec::new(),
            skipped_checks: Vec::new(),
            findings: Vec::new(),
            required_checks_done: Vec::new(),
            coverage: Vec::new(),
            candidates: Vec::new(),
            candidate_decisions: Vec::new(),
        };
        let spec = load_embedded_stage_spec(StageKind::Enumeration).unwrap();
        let result = validate_stage_gate_with_context(&deliverable, &spec, None, None, &ctx);
        let ip_gaps = result
            .recovery_actions
            .as_ref()
            .map(|recovery| {
                recovery
                    .coverage_gap_actions
                    .iter()
                    .filter(|gap| gap.asset == "https://203.0.113.10:443")
                    .map(|gap| gap.technique.as_str())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        assert_eq!(
            ip_gaps,
            HashSet::from([
                "GOLISH-ENUM-JS",
                "GOLISH-ENUM-DIR",
                "GOLISH-ENUM-PARAM",
                "GOLISH-ENUM-JSAPI",
            ]),
            "submit preview must expose four actionable gaps for an exact IP origin"
        );
    }

    // Fix1 (2026-06-14) · the parameters schema must spell out the nested shapes
    // the model kept guessing wrong — especially `skipped_checks[].reason` (the
    // SkipReason enum, internally tagged by `kind`), the coverage cell `status`
    // enum and the finding `severity` enum. A regression to opaque
    // `{"type":"object"}` items reintroduces the submit-reject retry loop.
    #[test]
    fn parameters_spell_out_nested_enum_shapes() {
        let (stage, sink) = handles();
        let tool = SubmitStageDeliverableTool::new(stage, sink);
        let p = tool.parameters();

        // skipped_checks.reason is a structured object with a `kind` enum listing
        // the real SkipReason variants (this is the exact field the model failed).
        let reason = &p["properties"]["skipped_checks"]["items"]["properties"]["reason"];
        assert_eq!(reason["type"], json!("object"));
        let kinds = reason["properties"]["kind"]["enum"]
            .as_array()
            .expect("reason.kind enum");
        for variant in [
            "other",
            "rate_limited",
            "scope_restriction",
            "env_unavailable",
            "user_requested",
        ] {
            assert!(
                kinds.iter().any(|v| v == variant),
                "reason.kind enum must list {variant}: {kinds:?}"
            );
        }

        // coverage cell status enum + claims/findings items are real objects.
        let status = &p["properties"]["coverage"]["items"]["properties"]["status"];
        for s in ["found", "checked_empty", "blocked", "not_applicable"] {
            assert!(
                status["enum"].as_array().unwrap().iter().any(|v| v == s),
                "coverage status enum must list {s}"
            );
        }
        // T1 (2026-06-23): coverage cell reason_kind enum lists the structured
        // reason categories so the model can classify blocked/not_applicable.
        let reason_kind = &p["properties"]["coverage"]["items"]["properties"]["reason_kind"];
        for rk in [
            "provider_missing",
            "credential_missing",
            "rate_limited",
            "tool_missing",
            "out_of_scope",
            "not_applicable",
        ] {
            assert!(
                reason_kind["enum"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v == rk),
                "coverage reason_kind enum must list {rk}"
            );
        }
        let severity = &p["properties"]["findings"]["items"]["properties"]["severity"];
        assert!(severity["enum"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "critical"));
        assert_eq!(
            p["properties"]["claims"]["items"]["type"],
            json!("object"),
            "claims items must be a typed object, not opaque"
        );
        // claims items must actually declare properties (not be an empty object).
        assert!(p["properties"]["claims"]["items"]["properties"]["evidence_ids"].is_object());
        let coverage_desc = p["properties"]["coverage"]["description"]
            .as_str()
            .expect("coverage description");
        assert!(
            coverage_desc.contains("SERVICE-FINGERPRINT tested_units = open ports fingerprinted"),
            "coverage description must include EAS denominator example: {coverage_desc}"
        );
        assert!(
            p["properties"]["coverage"]["items"]["properties"]["tested_units"]["description"]
                .as_str()
                .unwrap()
                .contains("open ports fingerprinted")
        );
    }

    // Fix7 (2026-06-14): a no-tool stage (scoping has empty expected_techniques)
    // that receives an invented coverage cell must be told to resubmit with
    // coverage: [] instead of looping on no-tool-stage coverage.
    #[tokio::test]
    async fn no_tool_stage_invented_coverage_gets_empty_coverage_hint() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let mut args = valid_scoping_args();
        args["coverage"] = json!([
            { "asset": "ACME", "technique": "scoping", "status": "found", "evidence_refs": [] }
        ]);
        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("needs_fix"));
        let reasons = out["reasons"].as_array().expect("reasons array");
        assert!(
            reasons
                .iter()
                .any(|r| r.as_str().unwrap_or("").contains("coverage: []")),
            "must hint to resubmit with empty coverage: {reasons:?}"
        );
    }

    // 2026-06-15-recon-stage-findings-suppression: a recon/discovery stage
    // (scoping, findings_allowed=false) DROPS any submitted findings — they never
    // reach the stored deliverable — and the accept note tells the model so.
    #[tokio::test]
    async fn recon_stage_drops_findings_and_notes_it() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let mut args = valid_scoping_args();
        args["findings"] = json!([{
            "finding_id": "11111111-1111-1111-1111-111111111111",
            "kind": "exposed_admin",
            "subject": "example.com",
            "severity": "high",
            "evidence_refs": [1]
        }]);
        let out = tool.execute(args, Path::new("/tmp")).await.unwrap();
        assert_eq!(out["status"].as_str(), Some("accepted"));
        assert!(
            out["note"].as_str().unwrap_or("").contains("DROPPED"),
            "accept note must mention dropped findings: {out:?}"
        );
        // Stashed deliverable was sanitized: no findings reach the stage-close gate.
        let captured = sink.read().await.clone().expect("captured");
        let parsed: StageDeliverable = serde_json::from_str(&captured).expect("parse stashed");
        assert!(
            parsed.findings.is_empty(),
            "findings must be dropped from the stashed recon deliverable"
        );
    }

    // VulnTriage is observation-only for every persisted attack contract. Only
    // legacy Verification may retain model-authored findings.
    #[test]
    fn attack_stage_findings_policy_is_persisted_contract_aware() {
        use golish_core::AttackExecutionContract::{
            DualWriteReadLegacy, DualWriteReadV2Fallback, Legacy, V2Only,
        };

        for contract in [Legacy, DualWriteReadLegacy, DualWriteReadV2Fallback] {
            assert!(!findings_allowed_for_attack_contract(
                StageKind::VulnTriage,
                true,
                contract
            ));
            assert!(findings_allowed_for_attack_contract(
                StageKind::Verification,
                true,
                contract
            ));
        }
        for stage in [
            StageKind::VulnTriage,
            StageKind::AttackCandidate,
            StageKind::Verification,
        ] {
            assert!(!findings_allowed_for_attack_contract(stage, true, V2Only));
        }
        assert!(findings_allowed_for_attack_contract(
            StageKind::Scoping,
            true,
            V2Only
        ));
    }

    #[test]
    fn vuln_stage_drops_findings() {
        let mut deliverable: StageDeliverable = serde_json::from_value(json!({
            "stage_id": "vuln_triage",
            "stage_run_id": "3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
            "claims": [],
            "evidence_refs": [1],
            "findings": [{
                "finding_id": "11111111-1111-1111-1111-111111111111",
                "kind": "sqli",
                "subject": "api.example.com",
                "severity": "high",
                "evidence_refs": [1]
            }]
        }))
        .expect("Vuln deliverable");

        assert_eq!(drop_disallowed_findings(&mut deliverable, false), 1);
        assert!(
            deliverable.findings.is_empty(),
            "vuln stage observations must not author findings"
        );
    }

    // No active stage (e.g. flag on but stage not yet published): the tool still
    // captures the deliverable and defers the decision to the gate hook.
    #[tokio::test]
    async fn received_when_no_active_stage() {
        let (stage, sink) = handles();
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let out = tool
            .execute(valid_scoping_args(), Path::new("/tmp"))
            .await
            .unwrap();
        assert_eq!(out["status"].as_str(), Some("received"));
        assert!(sink.read().await.is_some());
    }

    // ── Piece 3 · closeout reconciliation barrier ──────────────────────────

    /// Background-jobs mock: the runtime event wait either reconciles within
    /// its budget or returns the still-outstanding jobs at deadline.
    struct BgJobsMock {
        running: Vec<RunningJobInfo>,
        settles_within_wait: bool,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl BgJobsMock {
        fn always_running(n: usize) -> Self {
            Self {
                running: (0..n)
                    .map(|i| RunningJobInfo {
                        job_id: format!("job_{i}"),
                        command: format!("masscan -p- target{i}"),
                        elapsed_ms: 45_000,
                    })
                    .collect(),
                settles_within_wait: false,
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        fn settles_within_wait(n_jobs: usize) -> Self {
            let mut m = Self::always_running(n_jobs);
            m.settles_within_wait = true;
            m
        }
    }

    #[async_trait::async_trait]
    impl BackgroundJobsQuery for BgJobsMock {
        async fn wait_for_session_reconciled(
            &self,
            _session_id: &str,
            _max_wait: std::time::Duration,
        ) -> Vec<RunningJobInfo> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.settles_within_wait {
                Vec::new()
            } else {
                self.running.clone()
            }
        }
    }

    // A submit that arrives while the session still has backgrounded scans
    // running is DEFERRED: needs_fix listing the still-running jobs, with a
    // exceptional recovery guidance — and nothing is stashed (the stage is not
    // graded against half-landed evidence).
    #[tokio::test]
    async fn submit_deferred_when_background_jobs_running() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink))
            .with_session_id("pentest-chat-1")
            .with_background_jobs(Arc::new(BgJobsMock::always_running(2)))
            // wait budget 0 ⇒ single-shot check (the timeout branch) with no delay.
            .with_reconcile_timeouts(0, 1);

        let out = tool
            .execute(valid_scoping_args(), Path::new("/tmp"))
            .await
            .unwrap();
        assert_eq!(out["status"].as_str(), Some("needs_fix"));
        let jobs = out["running_background_jobs"]
            .as_array()
            .expect("running_background_jobs present");
        assert_eq!(jobs.len(), 2, "both running jobs are listed: {out:?}");
        let reason = out["reasons"][0].as_str().unwrap();
        assert!(reason.contains("deadline expired"), "reason: {reason}");
        assert!(reason.contains("Do NOT re-run"), "reason: {reason}");
        assert!(
            !reason.contains("wait_for_background_jobs"),
            "reason: {reason}"
        );
        assert!(
            reason.contains("at most once with check_job"),
            "reason: {reason}"
        );
        // Short-circuits BEFORE the gate preview ⇒ nothing captured.
        assert!(
            sink.read().await.is_none(),
            "a deferred submit must not stash a deliverable"
        );
    }

    // Once the backgrounded scans settle (within the wait budget), the barrier
    // lets the submit proceed to the normal gate preview → accepted.
    #[tokio::test]
    async fn submit_proceeds_after_background_jobs_settle() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink))
            .with_session_id("pentest-chat-1")
            .with_background_jobs(Arc::new(BgJobsMock::settles_within_wait(1)))
            .with_reconcile_timeouts(5_000, 1);

        let out = tool
            .execute(valid_scoping_args(), Path::new("/tmp"))
            .await
            .unwrap();
        assert_eq!(
            out["status"].as_str(),
            Some("accepted"),
            "once jobs settle the submit proceeds normally: {out:?}"
        );
        assert!(sink.read().await.is_some());
    }

    // Without a session_id the barrier cannot attribute jobs, so it stays inert
    // even when the manager reports running jobs (no false deferral).
    #[tokio::test]
    async fn reconciliation_barrier_inert_without_session_id() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink))
            .with_background_jobs(Arc::new(BgJobsMock::always_running(3)))
            .with_reconcile_timeouts(0, 1);

        let out = tool
            .execute(valid_scoping_args(), Path::new("/tmp"))
            .await
            .unwrap();
        assert_eq!(
            out["status"].as_str(),
            Some("accepted"),
            "no session id ⇒ barrier inert ⇒ normal accept: {out:?}"
        );
    }
}
