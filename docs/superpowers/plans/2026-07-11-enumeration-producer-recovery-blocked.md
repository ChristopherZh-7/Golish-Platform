# Enumeration producer recovery-exhausted blocked implementation plan

**Goal:** Allow only trusted, recovery-exhausted direct producers to close their
owned Enumeration axes as `blocked`, without weakening found/empty ownership or
current-target evidence guards.

## 1. Lock the contract with failing tests

- Add app evidence tests for browser recovery blocked on JS/JSAPI/PARAM and
  rejection on DIR or wrong evidence kind.
- Add kit org-gate tests for browser and route source/axis ownership.
- Extend StageAssetCoverage projection tests so the UI read model accepts the
  same narrow matrix and rejects wrong owners.
- Run the `recovery_blocked` filter and record the expected red failures before
  implementation.

## 2. Implement strict app-side audit projection

- In `golish-agent-app/src/ai/db_bridge/evidence.rs`, keep found/empty ownership
  unchanged.
- Admit blocked only for the three exact tool/kind/axis combinations in the
  design document.
- Preserve the existing fresh current-org/current-target/exact-origin audit
  query and positive evidence-id join.
- Reuse `projected_technique_outcome_evidence_id` so DB worklist, UI read model,
  submit preview, and final gate share one projection rule.

## 3. Implement kit-side defense in depth

- In `golish-agent-kit/src/harness/org_gate.rs`, replace the preflight-only
  blocked source check with the exact source/axis matrix.
- Continue requiring a positive evidence id and the exact canonical
  `(origin, technique, evidence_id)` blocked fact.
- Do not add DB/audit-kind knowledge to agent-kit; the app bridge owns that
  validation across the dependency-inversion seam.

## 4. Synchronize agent-facing contracts

- Update Enumeration `spec.json` and `methodology.md` with the authority table,
  ordinary non-terminal behavior, and no-retry rule for persisted
  recovery-exhausted blocked cells.
- Update compact worklist/status text and the Enumerator prompt.
- Update agent-app, agent-kit, sub-agent, and pentest bridge module cards.
- Document browser recovery exclusions and route checkpoint-v8 network,
  business-write, terminal-publication, generation-CAS, and manual-repair thresholds.

## 5. Verify

Run:

```bash
cd backend
CARGO_INCREMENTAL=0 cargo nextest run -p golish-agent-app -p golish-agent-kit recovery_blocked --status-level fail
CARGO_INCREMENTAL=0 cargo nextest run -p golish-agent-app ai::db_bridge::evidence::tests:: --status-level fail
CARGO_INCREMENTAL=0 cargo nextest run -p golish-agent-app ai::commands::stage_coverage::tests:: --status-level fail
CARGO_INCREMENTAL=0 cargo nextest run -p golish-agent-kit harness::org_gate::tests:: --status-level fail
CARGO_INCREMENTAL=0 cargo nextest run -p golish-agent-kit tool_executors::security::tests:: --status-level fail
CARGO_INCREMENTAL=0 cargo nextest run -p golish-sub-agents defaults --status-level fail
cargo clippy -p golish-agent-app -p golish-agent-kit -p golish-sub-agents --all-targets -- -D warnings
```

Also validate Enumeration JSON, Rust formatting, and whitespace. Full repository
precommit remains the parent integration task because the workspace contains
concurrent changes outside this narrow implementation.
