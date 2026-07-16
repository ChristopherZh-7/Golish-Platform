# `stage_run` terminal request control implementation plan

> Execute without `init.sh`, GUI, external LLM calls or real scanning. Run `just space-guard`
> before every Cargo build/test command.

## Task 1: Lock the failed behavior with headless tests

1. Add a dispatch-level test for the closed `stage_run.runtime_control` discriminator.
2. Add a scripted agent-loop test whose first response calls `stage_run` and whose second response
   would attempt post-block work.
3. Assert the RED behavior: the model is called twice before the production fix.

## Task 2: Add typed terminal dispatch control

1. Define a closed `ToolDispatchHaltReason` and carry it in `ToolDispatchOutcome`.
2. Parse only the first JSON value in the paired ToolResult, preserving support for runtime notes.
3. Generalize the existing accepted-submit batch barrier so terminal `stage_run` also skips later
   calls with paired synthetic results.
4. Break `run_turn_loop` immediately after recording the terminal ToolResult batch.

## Task 3: Author the server control signal

1. Add `runtime_control=halt_current_request/operator_recovery_required` to the Company Controller
   operator-recovery result.
2. Add `runtime_control=halt_current_request/stage_run_reentry_blocked` to the same-request reentry
   result.
3. Keep `success=true, passed=false` and all existing durable recovery fields unchanged.

## Task 4: Validate the CLI/headless closed loop

1. Run the new scripted agent-loop test and dispatch batch tests.
2. Run focused Company Controller/operator-recovery/reentry tests.
3. Run the relevant runtime agent-loop regression selection and scoped Clippy.
4. Run rustfmt check, JSON validation and diff hygiene.
5. Record exact commands, exit codes and nextest run ids in `agent-progress.md`; keep the broad
   feature `in_progress` unless its separate Candidate/Verification DoD is also complete.

## Task 5: Synchronize workspace truth

1. Update the `golish-agent-runtime/agentic_loop` module card and module index sync note.
2. Link this design and plan from the existing Stage Team feature.
3. Do not commit, stage or push unless the user separately requests it.

## Task 6: Keep Gate BLOCK visibly incomplete

1. Add a focused orchestrator test proving only Gate PASS may emit a subtask completion marker.
2. Suppress `SubtaskCompleted` and `completed_results` for blocked stage outcomes.
3. Replace the unconditional “Send a message to resume” promise with a saved-state/recovery-policy
   explanation.
