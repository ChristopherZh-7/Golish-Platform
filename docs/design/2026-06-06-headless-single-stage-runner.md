# Headless Single-Stage Harness Runner (方案 2)

> Status: draft (awaiting confirmation)
> Author: agent session bajie-mcp-agent-4
> Date: 2026-06-06
> Related: `docs/design/2026-06-06-intel-stage-ai-driven-per-mode.md`,
> `docs/design/2026-06-05-unified-ai-harness-observability.md`

## 1. Problem

Testing one harness stage today means: `just dev` (full Tauri GUI, minutes to
boot) + a real LLM key + manually creating an engagement + steering the agent
all the way from `scoping` down to the stage you care about + then reading
`~/.golish/backend.log` / `transcript.json` by hand to see what the gate did.

Slow, expensive, non-deterministic, and there is **no way to start at (or stop
after) a chosen stage**. The pain: *"I just want to test `target_intel`, or just
`scoping`."*

## 2. Goal / Non-goals

**Goal (方案 2):** a headless command that boots the real backend (embedded
Postgres + real pentest tools + real LLM) **with no GUI**, runs a chosen slice of
the stage DAG (down to / only a target stage), and prints a structured report
(tools called, deliverable, gate PASS/BLOCK + reasons, evidence booked) — then
exits.

**Non-goals:**
- Not a replacement for the deterministic gate unit tests (`harness/e2e_tests.rs`).
  Those stay; this is the *live* counterpart.
- Not a mocked-LLM harness (that is 方案 1, deferred).
- Not record/replay (方案 3, deferred).
- P0 does not seed arbitrary prior evidence; see §6 phasing.

## 3. Key architectural facts (verified)

| Fact | Evidence |
|---|---|
| One `tasks` row = one operation; `operation_state.operation_id == tasks.id` | `orchestrator.rs:136-163` |
| Stage cursor = `operation_state.current_stage` (Postgres), starts at `Scoping` | `orchestrator.rs:158-163` |
| Stage DAG = `base_operation_graph().project(profile.allowed_stage_set())` | `execute.rs:526-552` |
| Single entry = `dag.entry_points().first()`; PASS with 0 successors → Complete | explore: `operation_flow.rs:277`, `stage_transition.rs` |
| **`AppState::new` takes NO `AppHandle`** (settings/telemetry/pool/gate only) | `state/mod.rs:62-91` |
| `extract_agent_state()` yields `AgentState` w/ `pentest_tool_factory` | `state/mod.rs:127-146` |
| `configure_bridge(bridge, &agent_state, session, app_handle: Option)` wires pentest tools + DB repo + domain hooks; accepts `None` handle | `bridge_config.rs:18-60, 392-425` |
| GUI run entry = `execute_task_mode` (session row → `GolishDbRepoProvider` → `TaskOrchestrator` → `run/resume`) | `chat.rs:113-207` |
| Embedded PG is Tauri-free: `GolishDb::start` / `create_lazy_pool` + `db_ready` gate | `bootstrap.rs:116-186` |
| CLI today (`--headless`) builds a bridge but **never boots DB nor the orchestrator** | `cli/runner.rs:48`, `cli/bootstrap/mod.rs` |
| `StageKind::try_parse(&str)` / `as_str()` exist | `harness/types.rs:33,52` |

## 4. Design

### 4.1 Entry surface

New headless mode on the unified `golish` binary (mirrors `--replay`'s
short-circuit in `main.rs`), plus a `just` recipe.

```
golish --stage-run \
  --profile red_team \        # harness profile id (assessment/pentest/red_team/...)
  --to target_intel \         # run the DAG slice up to & incl. this stage, then stop
  [--from scoping] \          # default scoping (the DAG entry)
  [--only target_intel] \     # shorthand for --from X --to X (single stage; needs seed for non-scoping)
  [--org "ACME Corp"] \       # P0 minimal seed: create org (for stages that need one)
  [--target example.com] \    # P0 minimal seed: in-scope target(s), repeatable
  [--auto-approve] \          # auto-answer scoping HITL (scope_review/unit_review)
  [--json] \                  # machine-readable report
  "<task input / objective>"
```

`just stage profile to input` recipe wraps the above with `--auto-approve`.

### 4.2 Boot sequence (`run_stage_headless`, new module `golish/src/stage_run/`)

1. Settings + telemetry (reuse `cli::bootstrap` helpers / `init_settings_manager`).
2. `create_lazy_pool` + `DbReadyGate`; `spawn_embedded_pg`; **block** on
   `db_ready.wait_timeout(...)` (CLI must wait, unlike the GUI's lazy gate).
3. `AppState::new(settings, langfuse, stats, pool, db_ready)` →
   `agent_state = app_state.extract_agent_state()`.
4. Build provider bridge (reuse `initialize_agent`'s provider match), then
   `configure_bridge(&mut bridge, &agent_state, "stage-run-<uuid>", None)`.
5. `bridge.set_execution_mode(Task)` + `bridge.set_harness_profile(Some(profile))`.

### 4.3 DAG-slice control (new orchestrator seam)

Add to `TaskOrchestrator`:
- field `stage_allowlist: Option<HashSet<StageKind>>` + `set_stage_allowlist(..)`.
- In `run_executor_driven` projection (`execute.rs:536/552`), when the allowlist
  is `Some`, project with `allowed ∩ allowlist`. A slice `{scoping..=to}` yields a
  DAG whose terminal node is `to` (downstream edges dropped) → after `to` PASSes,
  0 successors → `Complete`, executor exits. **No core transition logic changes.**
- New entry `run_stage(from, allowlist, task_input, executor)` mirroring `run`
  but inserting `operation_state` at `from` (not always `Scoping`).

Slice computation: `allowlist = {s ∈ profile.allowed : s can-reach `to` along the
projected DAG}` (ancestors-of-`to` ∪ `{to}`), further intersected with
`{descendants-of-`from`}` when `--from` is given. For the linear profiles this is
just the prefix `scoping → … → to`.

### 4.4 Orchestration (mirror `execute_task_mode`, headless)

`sessions::upsert_by_chat_key` → `GolishDbRepoProvider` → `TaskOrchestrator::new`
→ `set_profile_override(profile)` + `set_chat_session_id(session)` +
`set_approval_coordinator(bridge.coordinator())` + **`set_stage_allowlist(slice)`**
→ `BridgeAgentExecutor::new(bridge)` → `run_stage(from, slice, input, executor)`.

### 4.5 Headless HITL auto-approve

`scoping` cannot pass its gate without a `scope_human_approved` claim, which comes
from an `ask_human(scope_review)` approval. Headless has no UI to click, so under
`--auto-approve` a small task subscribes to the bridge **coordinator**'s pending
`AskHumanRequest`s and auto-confirms them (approve scope as-proposed). Without
this, `--only scoping` / any slice through scoping would hang. (Intel has no HITL,
so intel-only runs do not need it.)

### 4.6 Minimal upstream seeding (implemented · P1a)

For `--only <stage>` where `stage != scoping`, the stage needs real upstream data
(e.g. `target_intel` needs an organization + in-scope targets, since `recon_*`
tools take `organization_id` and the gate's in-scope-asset axis is built from the
targets table). `seed_upstream` (in `stage_run`) creates the **minimum** via flags,
reusing existing repos:
- `--org "<name>"` → `organizations::create(pool, project_path=workspace, name, …)`.
- `--target <host>` (repeatable) → `PgReconTargetsAdapter::target_add(…, org_id,
  project_path=workspace, …)` (defaults `scope='in'`), bound to the org.

The seeded `organization_id` is injected into the run objective so the agent can
call `recon_*` without first guessing the org. Scope alignment (verified):
`in_scope_assets` ← `list_in_scope_values($1 IS NULL ⇒ every in-scope row, any
project_path)`, while `manage_targets`/`manage_organizations` scope by the same
workspace `project_path`.

Richer seeding (prior evidence ids, claims, multi-stage handoff) is **P1b** via a
`--seed <file.json>`.

> Easiest path that needs **zero** seeding: `--to target_intel` (default
> `--from scoping`) runs `scoping → target_intel` for real — scoping produces the
> org/targets that intel consumes. `--only` is the true-isolation path that needs
> the seed.

### 4.7 Report

Collect the orchestrator's `AiEvent` stream (already emitted): `TaskProgress`
(`stage_passed`), `HarnessTrace { GateDecision { allowed, reasons }, EvidenceBooked }`,
tool-call/observation events. After the run, render per stage:
- stage id + PASS/BLOCK + gate reasons / recovery actions,
- tools invoked (name, ok/err),
- evidence ids booked,
- final deliverable summary (claims/coverage counts).
`--json` emits the same as JSON lines for scripting. Deep dive still available via
`golish --replay <session>` (transcripts are written as usual).

## 5. Touch points

- `golish/src/cli/args.rs` — new flags (`stage_run`, `profile`, `from`, `to`, `only`, `org`, `target`).
- `golish/src/main.rs` — dispatch `--stage-run` before GUI/CLI branch.
- `golish/src/stage_run/` (new) — boot + orchestrate + report.
- `golish-agent-kit/.../orchestrator.rs` + `subtask_phases/execute.rs` — `stage_allowlist` field/setter + `run_stage` + projection intersect.
- `golish-agent-kit/.../harness/operation_graph.rs` — small `ancestors`/`reachable` helper for slice (or compute in stage_run).
- `justfile` — `stage` recipe.
- Reuse as-is: `AppState::new`, `extract_agent_state`, `configure_bridge`, `initialize_agent`, `GolishDbRepoProvider`, `TaskOrchestrator`, `BridgeAgentExecutor`.

## 6. Phasing

- **P0** (done): headless boot + DAG-slice (`--from/--to`, `--only`) + HITL
  auto-approve + report + `just stage`. Headline: `--only scoping` and
  `--to target_intel` end to end.
- **P1a** (done): `--org`/`--target` upstream seeding — `seed_upstream`
  (`organizations::create` + `PgReconTargetsAdapter::target_add`, scoped to the
  workspace `project_path`; `target_add` defaults `scope='in'`) so an isolated
  `--only target_intel` has a real org + in-scope targets; the seeded
  `organization_id` is injected into the objective so the agent can call `recon_*`
  directly. Alignment: the gate's `in_scope_assets` reads
  `list_in_scope_values($1 IS NULL ⇒ all in-scope rows)`, and `manage_targets`/
  `manage_organizations` scope by the same workspace `project_path`.
- **P1b** (future): `--seed <json>` for arbitrary upstream (prior evidence ids /
  claims / multi-stage handoff), `--from <non-scoping>` jumps with seeded handoff.
- **P2**: optional `--ephemeral` DB / auto-cleanup of the scratch operation.

## 7. Tests (deterministic, no live LLM — run in `nextest`)

- arg parsing (`--stage-run`, `--only` ⇒ from==to, repeatable `--target`).
- slice computation: ancestors-of-`to` ∩ profile; `--only` ⇒ `{stage}`; linear
  profile prefixes; `to` outside profile ⇒ error.
- orchestrator projection honors `stage_allowlist` (project to single node ⇒ that
  node is the only entry, no successors).
- report renderer over a synthetic `AiEvent` vector (PASS, BLOCK+reasons, evidence).

Live single-stage run = the manual E2E this feature *is* (needs LLM key); it
replaces the old "drive the GUI by hand" loop, it is not asserted in CI.

## 8. Risks

- **HITL coordinator API** for headless auto-answer (§4.5) — RESOLVED feasible:
  `CoordinatorHandle::resolve_approval(ApprovalDecision)` (`handle.rs:81`) + we own
  the `AiEvent` stream headlessly, so a task watching for
  `AiEvent::AskHumanRequest { request_id, .. }` (`event.rs:158`) can auto-resolve.
  Remaining detail: construct the right `ApprovalDecision` (approve scope as-proposed)
  per `input_type` (`scope_review` / `unit_review`).
- DB pollution: each run creates a real operation/task in the persistent embedded
  PG. Acceptable for P0 (print the operation id); `--ephemeral` is P2.
- Provider/key resolution reuses CLI settings; no new secret handling.
