# Plan · EAS worker evidence citation + service-fingerprint truth fixes

> Design: `docs/design/2026-07-02-eas-worker-evidence-and-service-fingerprint.md`
> Scope: fix the `external_attack_surface` prober retry loop (A/B/C/D/E).
> Split: **Phase 1 (A + C2 + E)** landed first (additive/safe). **Phase 2 (B + C1 + D)**
> is now implemented as well, but the whole feature remains `in_progress` until a
> compile+targeted-test+EAS-smoke cycle is allowed and recorded.

## Phase 1 — additive, safe (landing now)

### Task A1 · repo query for detailed recent evidence
File: `backend/crates/golish-db/src/repo/audit/mod.rs`
- Add `pub struct RecentEvidenceRow { id, tool_name, subject, technique, asset, outcome, kind, age_seconds }`
  (all `Option<String>` except `id: i64`, `age_seconds: Option<f64>`).
- Add `pub async fn recent_evidence_detailed_for_session(pool, session_id, limit) -> Result<Vec<RecentEvidenceRow>>`:
  `SELECT id, tool_name, NULLIF(details,'') AS subject, evidence_technique, evidence_asset,
   evidence_outcome, detail->>'kind' AS kind, EXTRACT(EPOCH FROM (NOW()-created_at))::double precision AS age_seconds
   FROM audit_log WHERE audit_role='evidence' AND session_id=$1 ORDER BY id DESC LIMIT $2`.
- `limit <= 0` → `Ok(vec![])`.

### Task A2 · trait method
File: `backend/crates/golish-agent-kit/src/db_traits/repo.rs`
- Add `async fn recent_evidence_detailed(&self, session_id: &str, limit: i64) -> anyhow::Result<Vec<serde_json::Value>>`
  with default `Ok(Vec::new())` (test doubles unaffected). Doc: newest-first real
  evidence rows with (tool/subject/technique/asset/outcome/kind/age) so a worker can
  map a real id to the claim it backs.

### Task A3 · app-layer impl
File: `backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs` (+ helper in `recon.rs` or `evidence.rs`)
- Implement `recent_evidence_detailed` on `GolishDbRepoProvider`: call
  `golish_db::repo::audit::recent_evidence_detailed_for_session`, map each row to a
  compact `json!({ "evidence_id": .., "tool": .., "subject": .., "technique": ..,
  "asset": .., "outcome": .., "kind": .., "age_seconds": .. })` (drop nulls).

### Task A4 · tool definition
File: `backend/crates/golish-tools/src/definitions/security_tools.rs`
- Add `FunctionDeclaration { name: "list_recent_evidence", ... }`. Description: read-only;
  returns the run's recent real evidence-ledger ids with tool/asset/technique context
  so you can cite them in claim `evidence_ids` / `evidence_refs`; call it BEFORE
  submit_stage_deliverable instead of guessing ids. Param `limit` (int, default 25,
  max 200).

### Task A5 · dispatch routing
Files:
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
  — add `"list_recent_evidence"` to `is_security_analysis_direct_tool` (+ its
  routing test assertion).
- `backend/crates/golish-agent-kit/src/tool_executors/security.rs` — add to `is_sec_tool`
  match; add a handler arm: resolve `limit`, require `session_id` (else error_result),
  call `repo.recent_evidence_detailed(session_id, limit)`, return
  `json!({ "recent_evidence": rows, "count": rows.len(), "contract": "cite these real
  evidence_id values in claim evidence_ids / top-level evidence_refs; never invent ids" })`.

### Task A6 · expose to agents
Files:
- `backend/crates/golish-sub-agents/src/defaults/builder/registry.rs` — add
  `"list_recent_evidence".into()` to `prober`, `enumerator`, `pentester` `.with_tools`.
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_list.rs` — add to
  `READ_ONLY_QUERY_TOOLS`; update the membership + forbidden-context assertions.
- `frontend/lib/tools.ts` — add `list_recent_evidence: "Reading recent evidence"`.

### Task A7 · prompt
File: `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`
- In the prober/enumerator evidence guidance: "Before submit_stage_deliverable, call
  `list_recent_evidence` to get this run's REAL evidence ids (with their tool/asset/
  technique). Put the ids whose tool output backs each claim into that claim's
  `evidence_ids` and the top-level `evidence_refs`. Never invent ids and never use
  submit as a way to discover missing ids."

### Task C2 · land nmap -sV service/version as a fingerprint row
File: `backend/crates/golish-pentest/src/output_store/targets.rs`
- In `store_fingerprints`, after the existing `webserver`/`technologies`/`cdn`/`os`
  writes, add: if `fields.get("service")` is informative (helper
  `is_informative_service(s)` → not in {tcpwrapped, unknown, open, tcpwrapped?, ""}
  case-insensitive), split into `(name, version)` via `parse_server_version`, and
  `fingerprints::upsert(..., category="service", name, version, 0.7, ev, None, source)`.
- Confidence 0.7; evidence json `{ "source": source, "raw": service }`.

### Task E · retry budget
File: `backend/crates/golish-agent-kit/src/task_orchestrator/types.rs`
- `MAX_REFLECTOR_RETRIES: usize = 3` → `5`; update the doc comment to note the EAS
  async-landing rationale (design 2026-07-02).

### Phase 1 verification (deferred per user instruction "don't run tests")
- `just check` (fmt + biome + typecheck + clippy + rust tests) — NOT run in this pass.
- Targeted: `cargo nextest run -p golish-db -p golish-agent-kit -p golish-tools
  -p golish-agent-runtime -p golish-sub-agents`.
- ts-rs: no new cross-IPC type (tool returns `Vec<Value>`), so no generated-type churn.
- Live EAS smoke on `~/golish-platform/Test1`: confirm the prober calls
  `list_recent_evidence` and cites real ids, and `nmap -sV` service rows appear in
  `fingerprints`.

## Phase 2 — implemented, verification pending

### Task B · make scan output-store landing observable to the submit barrier
File: `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- Implemented B1: inside a harness stage, `await` the structured-storage hook for
  registry tool output instead of `tokio::spawn`, so `fingerprints`/`ports` land
  before the tool result returns. Non-harness execution keeps the old spawned path.
- Verify: `golish-agent-runtime` tests + EAS smoke that submit no longer grades before
  `-sV` lands.

### Task C1 · exclude non-informative pseudo-services from SERVICE-FINGERPRINT truth
File: `backend/crates/golish-db/src/repo/coverage_truth.rs`
- In `ports_have_service_hint_sql`, require the service be informative:
  `NULLIF(lower(p->>'service'),'') IS NOT NULL AND lower(p->>'service') NOT IN
   ('tcpwrapped','unknown','open','filtered','closed')`, and do not let a bare
  `service=domain` on `port=53` satisfy SERVICE-FINGERPRINT without version/webserver/
  technology truth. Add unit tests mirroring the existing `coverage_truth` test style.
- Coupled with D so port-53-only IPs get a deterministic terminal path.

### Task D · terminal SERVICE cell for port-found-but-no-service IPs
Files: `backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs`
(`apply_eas_service_dependency`) + `backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs`
(`coverage_complete`).
- Implemented via shared DB truth, not frontend-only inference:
  - `coverage_truth::eas_service_not_applicable_assets` returns current-wave IP/CIDR
    assets whose only observed port is DNS/53 and which have no fingerprint row or
    strong service surface.
  - `stage_coverage.rs` threads that set into `apply_eas_service_dependency` and marks
    SERVICE `not_applicable` with a DNS/53 note.
  - `GateContext.not_applicable_coverage` carries the same `(asset, technique)` set
    through submit preview and per-org close gate; `coverage_complete` accepts it only
    when `NotApplicable` is an allowed terminal state.
- Preserve I8: this is "technique does not apply to this infra IP", not "checked empty
  without a scan".
- Verify: new `rule_engine` + `stage_coverage` unit tests; EAS smoke on the 13-IP case.

## Notes
- Compilation is still unverified in this pass (user asked not to run precommit / tests).
  Running the targeted Rust checks and then `just check` is the first follow-up when
  validation is allowed.
- This feature must not be flipped to `passing` without fresh verification evidence per
  AGENTS.md §3.
