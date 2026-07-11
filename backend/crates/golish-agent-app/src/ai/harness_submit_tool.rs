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
use tokio::sync::RwLock;
use uuid::Uuid;

use golish_agent_kit::db_traits::{StageAssetWaveView, TechniqueOutcomeFact};
use golish_agent_kit::harness::org_gate::{
    apply_technique_outcome_rows, eas_service_not_applicable_from_port_outcomes,
    extract_pass_token, stage_accepts_outcome_projection, stage_accepts_source_query_completion,
    stage_gate_expected_techniques, validated_enumeration_axis_from_coverage_snapshot,
};
use golish_agent_kit::harness::{
    load_embedded_stage_spec, validate_stage_gate_with_context, AttackCandidate, EvidenceFact,
    GateContext, GateContextBuilder, SourceQueryFact, StageDeliverable, StageKind,
};
use golish_core::Tool;

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

    /// P5.1 (设计 2026-07-02-attack-stage §3.7 · candidate persistence): upsert the
    /// deliverable's attack hypotheses into `attack_candidates` so the chain-wave
    /// controller can dedupe across waves and follow a→b→c lineage
    /// (`parent_finding_id`). This is a **deterministic handler write** (the tool
    /// captured a structured `candidates[]`, not a model prose claim), org-isolated
    /// (I2), idempotent by `(operation_id, target, hypothesis_hash)`. Returns the
    /// number persisted. Default no-op (test doubles / no-DB); a write failure is
    /// non-fatal — the impl logs a warn and returns what it managed to persist.
    async fn persist_attack_candidates(
        &self,
        operation_id: &str,
        organization_id: Option<Uuid>,
        candidates: &[AttackCandidate],
    ) -> usize {
        let _ = (operation_id, organization_id, candidates);
        0
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
}

/// A still-running background job attributed to the current session. The closeout
/// reconciliation barrier surfaces these so the agent doesn't conclude a stage
/// while its own backgrounded scans are still in flight.
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
    /// Background jobs still `Running` that were started by `session_id`.
    async fn running_for_session(&self, session_id: &str) -> Vec<RunningJobInfo>;
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

/// Tool that captures a structured [`StageDeliverable`] into the bridge
/// side-channel so the deterministic gate can validate it, regardless of which
/// agent (orchestrator or `reporter`) produced it.
pub struct SubmitStageDeliverableTool {
    /// Active harness stage (set per-subtask by the Task-mode executor).
    active_stage: Arc<RwLock<Option<StageKind>>>,
    /// Sink the Task-mode executor reads at stage close + appends to content.
    last_deliverable: Arc<RwLock<Option<String>>>,
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
    /// session still has backgrounded scans running defers before gate preview.
    /// Production defaults `reconcile_wait_ms` to 0 so the wait is a visible
    /// `wait_for_background_jobs` tool step; operators may opt back into bounded
    /// in-submit waiting with `GOLISH_SUBMIT_RECONCILE_WAIT_MS`.
    /// `None` ⇒ barrier disabled (tests / no DI).
    bg_jobs: Option<Arc<dyn BackgroundJobsQuery>>,
    /// Total time the reconciliation barrier waits for running jobs to settle
    /// before giving up and telling the agent to wait + resubmit. `0` ⇒ no wait
    /// (single-shot check). Production wires this from
    /// `GOLISH_SUBMIT_RECONCILE_WAIT_MS`.
    reconcile_wait_ms: u64,
    /// Poll interval while waiting for running jobs to settle.
    reconcile_poll_ms: u64,
}

impl SubmitStageDeliverableTool {
    pub fn new(
        active_stage: Arc<RwLock<Option<StageKind>>>,
        last_deliverable: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            active_stage,
            last_deliverable,
            evidence_repo: None,
            session_id: None,
            org_id_source: None,
            operation_id_source: None,
            bg_jobs: None,
            reconcile_wait_ms: 0,
            reconcile_poll_ms: 1000,
        }
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

    /// Configure the reconciliation barrier's wait budget / poll interval (ms).
    /// Production reads `GOLISH_SUBMIT_RECONCILE_WAIT_MS`; tests pass small values
    /// to exercise the timeout branch without real delay.
    pub fn with_reconcile_timeouts(mut self, wait_ms: u64, poll_ms: u64) -> Self {
        self.reconcile_wait_ms = wait_ms;
        self.reconcile_poll_ms = poll_ms.max(1);
        self
    }

    /// Closeout reconciliation barrier (Piece 3). Returns `Some(needs_fix json)`
    /// when, after waiting up to `reconcile_wait_ms`, the session STILL has
    /// background jobs running — so the caller short-circuits the submit and the
    /// agent waits via the explicit `wait_for_background_jobs` control tool
    /// instead of grading the stage against half-landed evidence. Returns `None`
    /// when the barrier is disabled or all jobs have settled (proceed normally).
    async fn reconcile_background_jobs(&self) -> Option<Value> {
        let (bg, sid) = (self.bg_jobs.as_ref()?, self.session_id.as_deref()?);
        let mut running = bg.running_for_session(sid).await;
        if running.is_empty() {
            return None;
        }
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(self.reconcile_wait_ms);
        while !running.is_empty() {
            let now = std::time::Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = deadline.duration_since(now).as_millis() as u64;
            let nap = self.reconcile_poll_ms.min(remaining).max(1);
            tokio::time::sleep(std::time::Duration::from_millis(nap)).await;
            running = bg.running_for_session(sid).await;
        }
        if running.is_empty() {
            return None;
        }
        tracing::info!(
            target: "harness::submit_tool",
            session_id = %sid,
            still_running = running.len(),
            "submit deferred: background scans still running at stage close"
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
                "{} background job(s) you launched are still running, so this stage's \
                 evidence has not fully landed yet. Do NOT re-run the same command. Call \
                 wait_for_background_jobs to wait visibly and read the completed job output \
                 tails; then build the final deliverable from those results and call \
                 submit_stage_deliverable again. If one has clearly hung (see elapsed_ms \
                 below — very long with no progress, e.g. a DNS AXFR / zone-transfer probe), \
                 inspect it once with check_job and kill_job it if stuck, then resubmit rather \
                 than letting one hung probe block the stage.",
                running.len()
            )],
            "running_background_jobs": jobs,
            "note": "call wait_for_background_jobs, inspect the completed job tails it returns, then resubmit."
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

    /// P5.1 · persist the deliverable's attack candidates (if any) to the DB via
    /// the evidence-repo seam. Requires a bound operation id (the persistence key)
    /// + the repo handle; otherwise a no-op. Non-fatal — persistence failure only
    ///   logs (the deliverable itself is still captured by the gate path).
    async fn persist_candidates_if_any(&self, deliverable: &StageDeliverable) {
        if deliverable.candidates.is_empty() {
            return;
        }
        let Some(repo) = self.evidence_repo.as_ref() else {
            return;
        };
        let operation_id = match &self.operation_id_source {
            Some(src) => (*src.read().await).map(|id| id.to_string()),
            None => None,
        };
        let Some(operation_id) = operation_id else {
            tracing::debug!(
                target: "harness::submit_tool",
                candidates = deliverable.candidates.len(),
                "attack candidates not persisted: no bound operation id"
            );
            return;
        };
        let org_id = match &self.org_id_source {
            Some(src) => *src.read().await,
            None => None,
        };
        let stored = repo
            .persist_attack_candidates(&operation_id, org_id, &deliverable.candidates)
            .await;
        tracing::info!(
            target: "harness::submit_tool",
            operation_id = %operation_id,
            submitted = deliverable.candidates.len(),
            stored,
            "attack candidates persisted"
        );
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
        let active_session_id = if stage == StageKind::Enumeration {
            Some(
                self.session_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|session_id| !session_id.is_empty())
                    .ok_or_else(|| {
                        "enumeration submit preview requires the active non-empty run/session id"
                            .to_string()
                    })?,
            )
        } else {
            self.session_id.as_deref()
        };
        let Some(repo) = self.evidence_repo.as_ref() else {
            if stage == StageKind::Enumeration {
                return Err(
                    "enumeration submit preview requires the trusted DB coverage repository"
                        .to_string(),
                );
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
        if stage == StageKind::Enumeration && org_id.is_none() {
            return Err(
                "enumeration submit preview requires the active bound organization".to_string(),
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
        let freshness_cutoff = stage_spec
            .as_ref()
            .is_some_and(|spec| spec.freshness_window)
            .then_some(stage_started_at)
            .flatten();
        if stage == StageKind::Enumeration && freshness_cutoff.is_none() {
            return Err(
                "enumeration submit preview requires the current stage_started_at freshness cutoff"
                    .to_string(),
            );
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
                None => match wave_cutoff {
                    Some(cutoff) => {
                        repo.in_scope_assets_created_before(Some(org_id), cutoff)
                            .await
                    }
                    None => repo.in_scope_assets(Some(org_id)).await,
                },
            };
            if !assets.is_empty() {
                facts.extend(
                    repo.db_truth_facts_with_run_start(Some(org_id), &assets, freshness_cutoff)
                        .await,
                );
                if let Some(cutoff) = wave_cutoff {
                    tracing::info!(
                        target: "harness::submit_tool",
                        stage = %stage.as_str(),
                        org_id = %org_id,
                        asset_count = assets.len(),
                        cutoff = %cutoff,
                        "using current-wave in-scope assets for submit preview"
                    );
                }
                in_scope_assets = assets;
            }
            // (3) T3 · authoritative口径补全: host-aware asset_types + dynamic
            //     expected_techniques (same source as the stage-close gate), so the
            //     preview和close对同一交付物给同一判定（消除预检假 PASS / close
            //     BLOCK 分歧）。Each query fail-safes to empty (prior behaviour).
            //     预检不做 subsidiary-inject（需 engagement threshold，预检 seam 不
            //     持有；authoritative stage-close 仍强制该维）。
            if authoritative {
                typed_assets = repo.in_scope_typed_assets(Some(org_id)).await;
                if let Some(current_wave_assets) = current_wave_asset_values.as_ref() {
                    let current_wave_assets = current_wave_assets
                        .iter()
                        .map(String::as_str)
                        .collect::<HashSet<_>>();
                    typed_assets.retain(|(asset, _)| current_wave_assets.contains(asset.as_str()));
                }
                let target_types = repo.in_scope_target_types(Some(org_id)).await;
                expected_techniques = stage_gate_expected_techniques(stage, &target_types);
            }
            if stage == StageKind::Enumeration {
                let snapshot = repo
                    .stage_asset_coverage(
                        org_id,
                        stage,
                        active_session_id,
                        freshness_cutoff,
                        current_wave_target_ids.clone(),
                        current_wave_asset_values.clone(),
                    )
                    .await
                    .map_err(|error| {
                        format!("enumeration exact-origin coverage snapshot failed: {error}")
                    })?;
                let snapshot = snapshot.ok_or_else(|| {
                    "enumeration exact-origin coverage snapshot is unavailable".to_string()
                })?;
                (in_scope_assets, typed_assets) =
                    validated_enumeration_axis_from_coverage_snapshot(
                        &snapshot,
                        org_id,
                        active_session_id,
                    )
                    .map_err(|error| {
                        format!("enumeration exact-origin coverage snapshot is invalid: {error}")
                    })?;
                authoritative_coverage_axis = true;
            }
            // (4) #4/E3: **始终**从 technique_outcomes union 进 facts（submit 预检；与
            //     execute.rs/org_gate 同源 dual-read）。additive + fail-safe，无灰度开关。
            //     run_id = chat session；outcome blocked→Error（gate 无 Blocked outcome）。
            if let Some(sid) = active_session_id {
                if stage_accepts_outcome_projection(stage, freshness_cutoff.is_some()) {
                    outcome_rows = repo
                        .technique_outcome_facts_fresh(org_id, sid, freshness_cutoff)
                        .await;
                }
                if stage == StageKind::ExternalAttackSurface {
                    not_applicable_coverage
                        .extend(eas_service_not_applicable_from_port_outcomes(&outcome_rows));
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
            // 方案 A (设计 2026-06-30-eas-domain-port-delegation): EAS host-aware
            // alias exclusion — drop assets whose resolved IP is already an
            // in-scope IP target so the submit preview matches the stage-close
            // gate (org_gate) and the read-only precheck. Orphan domains stay in
            // the axis but only LIVENESS applies.
            if stage == StageKind::ExternalAttackSurface && !in_scope_assets.is_empty() {
                let delegated: std::collections::HashSet<String> = repo
                    .eas_port_delegated_assets(Some(org_id))
                    .await
                    .into_iter()
                    .collect();
                if !delegated.is_empty() {
                    in_scope_assets.retain(|asset| !delegated.contains(asset));
                    typed_assets.retain(|(asset, _)| !delegated.contains(asset));
                }
            }
        }
        // Always apply the stage projection, even without org/session/cutoff. For
        // Enumeration an empty row set deliberately clears legacy/business facts.
        apply_technique_outcome_rows(stage, &mut facts, &outcome_rows);
        // 统一组装入口（设计 2026-06-23-unified-gate-context-builder）。
        let builder = GateContextBuilder::new()
            .typed_assets(typed_assets)
            .web_capable_assets(web_capable_assets)
            .not_applicable_coverage(not_applicable_coverage)
            .extend_evidence_facts(facts)
            .extend_source_queries(source_queries)
            .expected_techniques(expected_techniques);
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
                    "description": "Optional security findings (vulnerabilities). ONLY for vulnerability stages (vuln_triage / verification). Recon / discovery stages (scoping, target_intel, external_attack_surface, enumeration) take NO findings: omit findings or submit [] and record discoveries (hosts / services / exposures) as claims + coverage cells instead — any findings sent in those stages are DROPPED. Evidence ids are optional; DB/ledger truth is resolved by the backend.",
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
        let args = canonicalize_model_submit_args(args);
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
                         coverage[], skipped_checks[], required_checks_done[]) as tool \
                         arguments — do not describe the JSON in prose."
                    ),
                }));
            }
        };

        // stage_run_id is server-assigned: overwrite any model-supplied value with a
        // fresh random UUID before it reaches the gate preview, the side-channel sink,
        // or any log. The field is only logged + non-nil-checked downstream, never used
        // for evidence binding (which keys off evidence_ids), so generating it here
        // removes a weak model's ability to emit fabricated, patterned ids
        // (deepseek-v4-flash emitted incrementing bb07→bb08→… UUIDs that polluted the
        // gate logs). The authoritative per-stage-run id used for evidence isolation /
        // persistence is the harness's own, sourced separately.
        deliverable.stage_run_id = uuid::Uuid::new_v4();

        // P5.1 · persist attack hypotheses (attack_candidate / verification stages)
        // to `attack_candidates` so the chain-wave controller can dedupe across
        // waves + follow a→b→c lineage. Deterministic handler write (the tool
        // captured structured candidates, not model prose), idempotent by hash,
        // org-isolated. No-op unless the deliverable actually carries candidates
        // and an operation id is bound (⇒ zero change for the recon stages).
        self.persist_candidates_if_any(&deliverable).await;

        let active = *self.active_stage.read().await;
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
            // 甲 · structural/semantic gate preview (no DB). The authoritative
            // evidence-ledger cross-check runs at stage close in the gate hook.
            if let Ok(spec) = load_embedded_stage_spec(kind) {
                // Recon/discovery stages declare `findings_allowed=false`: their
                // deliverable is observations (`claims`) + a coverage matrix, NOT
                // vulnerabilities. Drop any `findings` a (weak) model dumped here so
                // junk never pollutes the stored deliverable or the stage-close gate;
                // the accept note tells the model to put discoveries in `claims`.
                // Design 2026-06-15-recon-stage-findings-suppression.
                let dropped_findings = if !spec.findings_allowed && !deliverable.findings.is_empty()
                {
                    let n = deliverable.findings.len();
                    deliverable.findings.clear();
                    n
                } else {
                    0
                };
                // Project the session's ledger evidence-facts into the gate so an
                // authoritative_found stage credits real `found` cells (the
                // per-org recon "never attempted" loop fix). Empty/no-DB ⇒ default
                // context = prior behaviour. T3: gray-switch also feeds host-aware
                // asset_types + dynamic expected_techniques so the preview matches
                // the stage-close口径 (env GOLISH_SUBMIT_PREVIEW_AUTHORITATIVE_CONTEXT=0 reverts).
                self.backfill_required_checks_done_from_evidence(&mut deliverable, &spec)
                    .await;
                let authoritative = golish_agent_kit::harness::feature_flags::submit_preview_authoritative_context_enabled();
                let ctx = match self.gate_context(kind, authoritative).await {
                    Ok(ctx) => ctx,
                    Err(reason) => {
                        return Ok(json!({
                            "status": "needs_fix",
                            "reasons": [reason],
                            "note": "the trusted current-wave context is invalid; repair/reset the wave before resubmitting."
                        }));
                    }
                };
                if spec.specialist.is_some() && extract_pass_token(&deliverable).is_some() {
                    if let Ok(json_str) = serde_json::to_string(&deliverable) {
                        *self.last_deliverable.write().await = Some(json_str);
                    }
                    let fabricated = self.fabricated_refs(&deliverable).await;
                    if fabricated.is_empty() {
                        return Ok(json!({
                            "status": "accepted",
                            "note": "stage_run pass_token captured; the final fan-out closeout gate will recompute it from org_stage_completions."
                        }));
                    }
                    let available = self.available_real_ids().await;
                    return Ok(json!({
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
                    }));
                }
                let result =
                    validate_stage_gate_with_context(&deliverable, &spec, None, None, &ctx);
                // Stash the canonical JSON regardless — the stage-close gate is
                // authoritative; a structural block still informs the agent now.
                if let Ok(json_str) = serde_json::to_string(&deliverable) {
                    *self.last_deliverable.write().await = Some(json_str);
                }
                if result.allowed {
                    if spec.expected_techniques.is_empty() && !deliverable.coverage.is_empty() {
                        let available = self.available_real_ids().await;
                        return Ok(json!({
                            "status": "needs_fix",
                            "reasons": [
                                "This stage declares NO expected techniques and runs no tools, so it has \
                                 no coverage matrix. Resubmit with coverage: [] (remove the invented \
                                 cells)."
                            ],
                            "available_evidence_ids": available,
                            "note": "fix these and call submit_stage_deliverable again."
                        }));
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
                        return Ok(json!({ "status": "accepted", "note": note }));
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
                    return Ok(json!({
                        "status": "needs_fix",
                        "reasons": [reason],
                        "fabricated_evidence_refs": fabricated,
                        "available_evidence_ids": available,
                        "note": "fix these and call submit_stage_deliverable again."
                    }));
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
                return Ok(response);
            }
        }

        // No active stage / spec unavailable: still stash; the gate hook decides.
        if let Ok(json_str) = serde_json::to_string(&deliverable) {
            *self.last_deliverable.write().await = Some(json_str);
        }
        Ok(json!({ "status": "received" }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// P5.1 · a submitted deliverable carrying attack candidates persists them
    /// through the evidence-repo seam (with a bound operation id). Recon stages
    /// (no candidates) never touch the store.
    #[tokio::test]
    async fn submit_persists_attack_candidates_when_present() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CandidateStoreMock {
            persisted: Arc<AtomicUsize>,
        }
        #[async_trait::async_trait]
        impl EvidenceLedgerQuery for CandidateStoreMock {
            async fn existing_evidence_ids(&self, _ids: &[i64]) -> Result<HashSet<i64>> {
                Ok(HashSet::new())
            }
            async fn persist_attack_candidates(
                &self,
                _operation_id: &str,
                _organization_id: Option<Uuid>,
                candidates: &[AttackCandidate],
            ) -> usize {
                self.persisted.fetch_add(candidates.len(), Ordering::SeqCst);
                candidates.len()
            }
        }

        let (stage, sink) = handles();
        let persisted = Arc::new(AtomicUsize::new(0));
        let op_src: Arc<RwLock<Option<Uuid>>> = Arc::new(RwLock::new(Some(Uuid::new_v4())));
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(CandidateStoreMock {
                persisted: Arc::clone(&persisted),
            }))
            .with_operation_id_source(op_src);

        let args = json!({
            "stage_id": "attack_candidate",
            "stage_run_id": "3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
            "claims": [],
            "evidence_refs": [],
            "findings": [],
            "candidates": [{
                "candidate_id": "3f8a1c2e-1d4b-4e6a-9b2c-7a1e5f0c9d33",
                "target": "api.example.com",
                "hypothesis": "IDOR on /orders/{id}",
                "rationale": "sequential ids observed"
            }]
        });
        let _ = tool.execute(args, Path::new(".")).await;
        assert_eq!(
            persisted.load(Ordering::SeqCst),
            1,
            "one candidate must be persisted through the store seam"
        );
    }

    /// A deliverable with no candidates (e.g. a recon stage) must not invoke the
    /// candidate store at all — zero churn for the four info-gathering stages.
    #[tokio::test]
    async fn submit_without_candidates_skips_the_store() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingStore {
            calls: Arc<AtomicUsize>,
        }
        #[async_trait::async_trait]
        impl EvidenceLedgerQuery for CountingStore {
            async fn existing_evidence_ids(&self, _ids: &[i64]) -> Result<HashSet<i64>> {
                Ok(HashSet::new())
            }
            async fn persist_attack_candidates(
                &self,
                _operation_id: &str,
                _organization_id: Option<Uuid>,
                _candidates: &[AttackCandidate],
            ) -> usize {
                self.calls.fetch_add(1, Ordering::SeqCst);
                0
            }
        }

        let (stage, sink) = handles();
        let calls = Arc::new(AtomicUsize::new(0));
        let op_src: Arc<RwLock<Option<Uuid>>> = Arc::new(RwLock::new(Some(Uuid::new_v4())));
        let tool = SubmitStageDeliverableTool::new(stage, sink)
            .with_evidence_repo(Arc::new(CountingStore {
                calls: Arc::clone(&calls),
            }))
            .with_operation_id_source(op_src);

        let _ = tool.execute(valid_scoping_args(), Path::new(".")).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "no candidates ⇒ the candidate store is never called"
        );
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
        async fn source_query_facts(
            &self,
            _org_id: uuid::Uuid,
            _run_id: &str,
        ) -> Vec<SourceQueryFact> {
            self.source_queries.clone()
        }
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
            source_queries: vec![SourceQueryFact {
                source: "dig".into(),
                query: "dns_resolve".into(),
                target: "pingan.com".into(),
                technique: Some("GOLISH-INTEL-DNS".into()),
                status: "found".into(),
                evidence_ids: vec![100],
            }],
        };
        let org_src = Arc::new(RwLock::new(Some(uuid::Uuid::new_v4())));
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
                blocked("GOLISH-INTEL-OSINT")
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
        let repo = DbTruthMock {
            existing: [100, 101].into_iter().collect(),
            db_facts: vec![
                found("GOLISH-INTEL-DNS"),
                found("GOLISH-INTEL-WHOIS"),
                found("GOLISH-INTEL-ASN"),
                found("GOLISH-INTEL-CT"),
                found("GOLISH-INTEL-SUBDOMAIN"),
                found("GOLISH-INTEL-OSINT"),
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
        let org_src = Arc::new(RwLock::new(Some(uuid::Uuid::new_v4())));
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

    // 2026-06-15-recon-stage-findings-suppression: a vulnerability stage
    // (vuln_triage, findings_allowed=true) RETAINS findings — drop only applies to
    // recon stages. Asserted on the stashed deliverable regardless of gate outcome.
    #[tokio::test]
    async fn vuln_stage_keeps_findings() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::VulnTriage);
        let tool = SubmitStageDeliverableTool::new(stage, Arc::clone(&sink));

        let args = json!({
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
        });
        let _ = tool.execute(args, Path::new("/tmp")).await.unwrap();
        let captured = sink.read().await.clone().expect("captured");
        let parsed: StageDeliverable = serde_json::from_str(&captured).expect("parse stashed");
        assert_eq!(parsed.findings.len(), 1, "vuln stage must keep findings");
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

    /// Background-jobs mock: reports `running` for its first `running_polls`
    /// calls, then reports none — simulating scans that finish mid-wait. Set
    /// `running_polls = usize::MAX` to simulate jobs that never settle.
    struct BgJobsMock {
        running: Vec<RunningJobInfo>,
        running_polls: usize,
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
                running_polls: usize::MAX,
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        fn settles_after(n_jobs: usize, polls: usize) -> Self {
            let mut m = Self::always_running(n_jobs);
            m.running_polls = polls;
            m
        }
    }

    #[async_trait::async_trait]
    impl BackgroundJobsQuery for BgJobsMock {
        async fn running_for_session(&self, _session_id: &str) -> Vec<RunningJobInfo> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < self.running_polls {
                self.running.clone()
            } else {
                Vec::new()
            }
        }
    }

    // A submit that arrives while the session still has backgrounded scans
    // running is DEFERRED: needs_fix listing the still-running jobs, with a
    // message that tells the model to wait visibly via wait_for_background_jobs
    // and not rerun the same scan — and nothing is stashed (the stage is not
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
        assert!(reason.contains("still running"), "reason: {reason}");
        assert!(reason.contains("Do NOT re-run"), "reason: {reason}");
        assert!(
            reason.contains("wait_for_background_jobs"),
            "reason: {reason}"
        );
        assert!(reason.contains("output tails"), "reason: {reason}");
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
            // Running for the first 2 polls, then settles.
            .with_background_jobs(Arc::new(BgJobsMock::settles_after(1, 2)))
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
