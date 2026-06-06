# Plan: Headless Single-Stage Harness Runner (方案 2, P0)

> Design: `docs/design/2026-06-06-headless-single-stage-runner.md`
> Date: 2026-06-06

## Outcome

`golish --stage-run --profile <p> --to <stage> [--from <s>|--only <s>] [--org][--target]
[--auto-approve] [--json] "<input>"` (+ `just stage`) boots the real backend
headless, runs a DAG slice, and prints a structured report; full logs
(`backend.log` + `transcript.json`) are retained for `--replay` / GUI viewing.

Headline working paths: `--only scoping` and `--to target_intel` (from scoping).

## Tasks

### T1 · DAG slice helper (deterministic) — `harness/operation_graph.rs`
- `AllowedDag::descendants_inclusive(start)` / `ancestors_inclusive(target)` (BFS).
- `AllowedDag::slice(from: Option<StageKind>, to: StageKind) -> Result<HashSet<StageKind>, SliceError>`
  = `ancestors_inclusive(to)` (∩ `descendants_inclusive(from)` if `from` given).
  Errs: `ToNotInDag`, `FromNotInDag`, `FromCannotReachTo`.
- Unit tests: `--only` ⇒ {stage}; `--to target_intel` ⇒ {scoping,target_intel};
  `--to external_attack_surface` ⇒ prefix; `--to reporting` ⇒ all; errors.
- **Verify:** `cargo nextest run -p golish-agent-kit operation_graph`.

### T2 · Orchestrator seam — `task_orchestrator/orchestrator.rs` + `subtask_phases/execute.rs`
- Field `stage_allowlist: Option<HashSet<StageKind>>` + `set_stage_allowlist(..)`.
- `run_executor_driven`: when set, project with `allowed ∩ allowlist`.
- Extract `run` body to `run_from_stage(input, executor, entry: StageKind)`;
  `run` = `run_from_stage(.., Scoping)`; new `run_stage(entry, input, executor)`
  inserts `operation_state` at `entry`.
- Unit test: allowlist={target_intel} ⇒ projected DAG is single node.
- **Verify:** `cargo nextest run -p golish-agent-kit` green.

### T3 · CLI args + dispatch — `golish/src/cli/args.rs`, `golish/src/main.rs`
- Flags: `--stage-run` (bool), `--profile`, `--from`, `--to`, `--only`,
  `--org`, `--target` (repeatable `Vec<String>`). `--only X` ⇒ from==to==X.
- `main.rs`: dispatch `--stage-run` before the GUI/CLI branch (like `--replay`).
- Unit tests: parse `--stage-run --only`, repeatable `--target`.
- **Verify:** `cargo nextest run -p golish args`.

### T4 · stage_run module — `golish/src/stage_run/`
- `boot.rs`: settings+telemetry; `create_lazy_pool`+`spawn_embedded_pg`+wait gate;
  `AppState::new`; provider bridge (reuse `initialize_agent`); `configure_bridge(.., None)`;
  `set_execution_mode(Task)` + `set_harness_profile(profile)`.
- `seed.rs`: `--org`/`--target` ⇒ create org + in-scope targets (existing repos/tools).
- `run.rs`: compute slice (T1) + entry; mirror `execute_task_mode`; `set_stage_allowlist`;
  `run_stage(entry, input, executor)`; spawn HITL auto-approver (watch `AskHumanRequest`
  → `coordinator.resolve_approval`).
- `report.rs`: consume `AiEvent` stream → per-stage PASS/BLOCK+reasons / tools / evidence;
  `--json` lines. Unit test renderer over a synthetic event vec.
- **Verify:** `cargo check -p golish`; report renderer test.

### T5 · justfile recipe
- `stage profile to input:` → `cargo run -p golish -- --stage-run --profile {{profile}} --to {{to}} --auto-approve "{{input}}"`.

### T6 · Verify + close
- `just lint-rust` (clippy -D), `cargo nextest run` (kit + golish), `cargo fmt --check`, `ReadLints`.
- Manual live E2E (needs LLM key): `just stage pentest scoping "..."` / `just stage red_team target_intel "..."` — record in agent-progress.
- Update `agent-progress.md` + `feature_list.json` evidence.

## Risks / notes
- `--only <non-scoping>` needs org/target/project_path seeding + gate `GateContext`;
  if fiddly, P0 ships `--only scoping` + `--to <stage>` (no seed) solid, defers
  isolated downstream `--only` to P1.
- Don't change core transition logic; allowlist is a projection-time intersect only.
- HITL auto-approve only under `--auto-approve`.
