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
//! "describe" — it must fill `stage_id` / `claims` / `evidence_refs` fields),
//! which the handler captures deterministically into the bridge side-channel
//! (`harness_last_deliverable`). The Task-mode executor then feeds it to the
//! gate at stage close. See `docs/design/2026-06-02-submit-stage-deliverable-tool.md`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use golish_agent_kit::harness::{
    load_embedded_stage_spec, validate_stage_gate, StageDeliverable, StageKind,
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

    /// The operation's REAL evidence ids (newest first) to suggest when the
    /// model cited fabricated refs. Empty when no repo / no session / infra
    /// error (the caller degrades to a generic "run the tools first" message).
    async fn available_real_ids(&self) -> Vec<i64> {
        let (Some(repo), Some(sid)) = (self.evidence_repo.as_ref(), self.session_id.as_deref())
        else {
            return Vec::new();
        };
        repo.recent_evidence_ids(sid, 25).await.unwrap_or_default()
    }

    /// Cross-check the deliverable's `evidence_refs` against the real ledger.
    /// Returns the cited ids that do NOT exist (fabricated), in cited order.
    /// An infra error is treated as "can't prove fabrication" → empty (the
    /// authoritative stage-close gate still runs), mirroring the orchestrator's
    /// fail-open behaviour so DB blips never wedge a legitimate stage.
    async fn fabricated_refs(&self, deliverable: &StageDeliverable) -> Vec<i64> {
        let Some(repo) = self.evidence_repo.as_ref() else {
            return Vec::new();
        };
        let cited: Vec<i64> = deliverable
            .evidence_refs
            .iter()
            .map(|e| e.as_i64())
            .collect();
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
         text) — stage_id, a random uuid v4 stage_run_id, claims, evidence_refs, and \
         findings. The deterministic gate validates it against the evidence ledger to \
         advance the stage. This is the ONLY way to complete a stage. Do NOT hunt for \
         evidence ids in raw tool output — if your evidence_ids are missing or wrong, \
         this tool returns the operation's real evidence ids (`available_evidence_ids`) \
         so you can resubmit citing them."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "stage_id": {
                    "type": "string",
                    "description": "The current stage id, e.g. \"external_attack_surface\"."
                },
                "stage_run_id": {
                    "type": "string",
                    "description": "A random UUID v4 for this stage run."
                },
                "claims": {
                    "type": "array",
                    "description": "Observations, each {kind, subject, summary, evidence_ids:[int]}; every evidence_id must also appear in evidence_refs.",
                    "items": { "type": "object" }
                },
                "evidence_refs": {
                    "type": "array",
                    "description": "All evidence-ledger ids cited by claims/findings (>= the sum of minimum tool invocations).",
                    "items": { "type": "integer" }
                },
                "findings": {
                    "type": "array",
                    "description": "Findings, each {finding_id:uuid, kind, subject, severity, evidence_refs:[int]}.",
                    "items": { "type": "object" }
                },
                "skipped_checks": {
                    "type": "array",
                    "description": "Deliberately skipped checks, each {check, reason}. \"checked-empty\" is NOT the same as \"unchecked\".",
                    "items": { "type": "object" }
                },
                "required_checks_done": {
                    "type": "array",
                    "description": "Names of the required tools you actually ran (e.g. dns_resolve, http_probe).",
                    "items": { "type": "string" }
                }
            },
            "required": ["stage_id", "stage_run_id", "claims", "evidence_refs", "findings"]
        })
    }

    async fn execute(&self, args: Value, _workspace: &Path) -> Result<Value> {
        // Force structured emission: parse the args into the canonical type. A
        // prose / malformed submission is rejected with actionable feedback so
        // the model retries with real fields (immediate-feedback = option 甲).
        let deliverable: StageDeliverable = match serde_json::from_value(args) {
            Ok(d) => d,
            Err(e) => {
                return Ok(json!({
                    "status": "rejected",
                    "reason": format!(
                        "could not parse StageDeliverable: {e}. Pass the structured fields \
                         (stage_id, stage_run_id, claims[], evidence_refs[], findings[]) as tool \
                         arguments — do not describe the JSON in prose."
                    ),
                }));
            }
        };

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
            // 甲 · structural/semantic gate preview (no DB). The authoritative
            // evidence-ledger cross-check runs at stage close in the gate hook.
            if let Ok(spec) = load_embedded_stage_spec(kind) {
                let result = validate_stage_gate(&deliverable, &spec, None);
                // Stash the canonical JSON regardless — the stage-close gate is
                // authoritative; a structural block still informs the agent now.
                if let Ok(json_str) = serde_json::to_string(&deliverable) {
                    *self.last_deliverable.write().await = Some(json_str);
                }
                if result.allowed {
                    // P2 · validate-on-submit: structure passing is necessary but
                    // NOT sufficient. Cross-check evidence_refs against the real
                    // ledger NOW so a deliverable citing fabricated ids gets an
                    // immediate, actionable `needs_fix` instead of a misleading
                    // `accepted` (which makes the agent advance before the
                    // stage-close gate blocks it on the same fabrication).
                    let fabricated = self.fabricated_refs(&deliverable).await;
                    if fabricated.is_empty() {
                        return Ok(json!({
                            "status": "accepted",
                            "note": "structure OK and all cited evidence exists in the ledger; \
                                     the final evidence gate runs at stage close."
                        }));
                    }
                    // 乙 · don't just scold — name the REAL evidence ids this
                    // operation already has so the model fills them in instead of
                    // re-copying placeholders (the recurring weak-model failure).
                    let available = self.available_real_ids().await;
                    let reason = if available.is_empty() {
                        format!(
                            "cited evidence ids {fabricated:?} do not exist in the evidence ledger. \
                             No real evidence ids are recorded for this operation yet — run this \
                             stage's required tools first, then call submit_stage_deliverable again; \
                             this tool reports the operation's real evidence ids for you to cite. \
                             Never invent or copy placeholder ids like 1, 2, 3."
                        )
                    } else {
                        format!(
                            "cited evidence ids {fabricated:?} do not exist in the evidence ledger. \
                             Cite ONLY from the REAL evidence ids already recorded for this \
                             operation (newest first): {available:?}. Pick the ones whose tool \
                             output backs each claim and put them in BOTH the claim's evidence_ids \
                             and the top-level evidence_refs. Never invent or copy placeholder ids."
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
                // F1 · a structural/vacuous block is most often the agent
                // submitting empty/insufficient `evidence_ids` because it could
                // not locate them. Surface the operation's REAL ledger ids here
                // too (not only on the fabricated-ref branch above) so the single
                // reliable id source is always handed back at the point of
                // failure — instead of the agent hunting for an id field that the
                // sub-agent tool path never carries.
                let available = self.available_real_ids().await;
                let mut reasons = result.reasons;
                if !available.is_empty() {
                    reasons.push(format!(
                        "This operation's REAL evidence ids (newest first) are {available:?}. \
                         Put the ones whose tool output backs each claim into BOTH that claim's \
                         evidence_ids and the top-level evidence_refs, then resubmit. Never invent ids."
                    ));
                }
                return Ok(json!({
                    "status": "needs_fix",
                    "reasons": reasons,
                    "available_evidence_ids": available,
                    "note": "fix these and call submit_stage_deliverable again."
                }));
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
    }

    impl MockLedger {
        fn existing(ids: HashSet<i64>) -> Self {
            Self {
                existing: ids,
                recent: Vec::new(),
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
    // "run the tools first" wording), still rejecting the fabricated ref.
    #[tokio::test]
    async fn fabricated_needs_fix_without_session_degrades() {
        let (stage, sink) = handles();
        *stage.write().await = Some(StageKind::Scoping);
        let ledger = MockLedger {
            existing: HashSet::new(),
            recent: vec![88, 86],
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
            reason.contains("No real evidence ids are recorded"),
            "degraded reason instructs running tools first: {reason}"
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
        // A real-id hint reason is appended naming the ids.
        let reasons = out["reasons"].as_array().expect("reasons array");
        assert!(
            reasons.iter().any(|r| {
                let s = r.as_str().unwrap_or("");
                s.contains("644") && s.contains("646")
            }),
            "a reason names the real ids: {reasons:?}"
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
}
