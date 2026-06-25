# Stage-Aware DB-Backed Refiner Plan

Design: `docs/design/2026-06-25-stage-aware-db-refiner.md`.

Goal: replace Mentor-style repeated-tool intervention with a single stage-aware, DB-backed refiner path that triggers after gate/submit failures and emits structured repair directives.

## Implementation Status

2026-06-25 first implementation slice is in place:

- Mentor active intervention is disabled in main-agent and sub-agent tool-result paths; repeated-tool monitoring now emits telemetry only.
- `RepairDirective` / `RefinerContext` / deterministic StageRefiner live in `golish-agent-kit::task_orchestrator::stage_refiner`.
- `submit_stage_deliverable needs_fix` and `stage_run` per-org BLOCK both route through StageRefiner before producing `SubmitRepairMode`.
- Agent-run checkpoints persist both `submit_repair_mode` and the full `repair_directive`.
- EAS coverage-gap directive emits exact actions, liveness `not_applicable` guidance, service-fingerprint denominator guidance, broad-tool forbids, and narrow command hints.
- TargetIntel gets a provider-only repair path that forbids scan fallback.
- `HarnessTraceKind::StageRefinerDecision` and `scripts/run_tree.py` rendering are implemented.
- Bounded RefinerLLM remains intentionally unimplemented.

## Tasks

- [x] Task 1: Disable Mentor active intervention in repair paths
  - In sub-agent and stage-run repair mode, stop `EXECUTION ADVISOR` / `EXECUTION SUPERVISOR` injection.
  - Ensure hard mentor can no longer block same-batch tools during submit repair.
  - Keep repeated-tool telemetry only if it does not alter agent behavior.
  - Add regression coverage for "repair mode ignores mentor advice".

- [x] Task 2: Add `RepairDirective` DTOs
  - Add a structured directive type with stage, org, repair kind, root cause, actions, submit guidance, allowed/forbidden tools, evidence ids, and hashes.
  - Add JSON serialization for checkpoint persistence.
  - Add a compatibility conversion from `RepairDirective` to current `SubmitRepairMode`.

- [x] Task 3: Add StageRefiner interface and dispatcher
  - Introduce a common `StageRefiner` / `StageRefinerRegistry`.
  - Inputs include stage, org, agent path, gate reasons, submit result, available evidence ids, coverage gap actions, recent tool summary, and DB tracker.
  - First implementation may be deterministic only; no LLM escalation yet.

- [x] Task 4: Wire submit `needs_fix` through StageRefiner
  - In `submit_stage_deliverable needs_fix`, build a `RefinerContext`.
  - Produce and persist `RepairDirective` in `operation_state.state_blob.agent_run`.
  - Render the directive into the next-turn runtime correction.
  - Derive current `SubmitRepairMode` from the directive so existing tool guards still work.

- [x] Task 5: Wire `stage_run` per-org BLOCK through StageRefiner
  - Replace the current per-org `submit_coverage_gap_repair_mode_from_reasons` shortcut with StageRefiner output.
  - Persist the directive in the same per-org `agent_path` checkpoint.
  - Resume specialists from the directive instead of generic gate feedback.

- [x] Task 6: Implement EASRefinerCore
  - Query in-scope targets, ports, fingerprints, real_ip relationships, recent active evidence, background jobs, and freshness.
  - Convert `coverage_gap_actions` into exact repair actions.
  - Add policy for non-resolving domain liveness: prefer `not_applicable` with note/evidence until active Empty facts are first-class.
  - Add service-fingerprint denominator guidance for open ports.

- [x] Task 7: Add ToolSkill loading
  - Load stage methodology and StageSpec for EAS.
  - Load tool registry metadata for `httpx`, `naabu`, `nmap`, `whatweb`.
  - Add a small curated EAS tool skill table if existing metadata is insufficient.
  - Use ToolSkill to choose narrow command hints and forbidden broad patterns.

- [x] Task 8: Add observability
  - Add `HarnessTraceKind::StageRefinerDecision`.
  - Show refiner decisions in `scripts/run_tree.py`.
  - Include root cause preview, action count, gap count, whether LLM escalation was used, and directive hash.

- [x] Task 9: Add TargetIntelRefinerCore
  - Query `source_query_log`, `target_assets`, `dns_records`, RDAP/WHOIS rows, and provider status.
  - Emit provider-only repairs and prevent scan-tool fallback where the stage spec disallows scan tools.

- [ ] Task 10: Optional bounded RefinerLLM
  - Trigger only on repeated unchanged gaps, ambiguous terminal status, or final retry.
  - Input compact DB/tool/gate summary, not full transcript.
  - Parse output into `RepairDirective`; reject prose-only output.

- [x] Task 11: Retire remaining Mentor intervention
  - Remove or disable `mentor_one_shot` from active tool response paths.
  - Remove hard supervisor same-batch blocking.
  - Keep or delete telemetry based on observed value.

- [x] Task 12: Module docs and progress
  - Update module cards for `golish-agent-kit/task_orchestrator`, `golish-agent-runtime/agentic_loop`, `golish-sub-agents/executor`, and relevant stage/tool docs.
  - Update `feature_list.json` and `agent-progress.md`.

## Verification

- `cd backend && cargo nextest run -p golish-agent-runtime mentor submit_repair stage_run --status-level fail`
- `cd backend && cargo nextest run -p golish-sub-agents submit_repair coverage_gap --status-level fail`
- `cd backend && cargo nextest run -p golish-agent-kit refiner stage_refiner --status-level fail`
- `cd backend && cargo nextest run -p golish-agent-app submit_stage_deliverable --status-level fail`
- `python3 scripts/run_tree.py --workspace <ws> <session> --db` on an EAS repair run, confirming StageRefiner decisions appear and Mentor does not inject conflicting advice.
- 2026-06-25 scoped build: `cd backend && cargo check -p golish-core -p golish-events -p golish-sub-agents -p golish-agent-kit -p golish-agent-runtime` → exit 0.

## Non-Goals

- No DB schema migration in the first implementation slice.
- No model-decided gate PASS/BLOCK.
- No refiner-synthesized deliverable.
- No full `just precommit` unless explicitly requested.
