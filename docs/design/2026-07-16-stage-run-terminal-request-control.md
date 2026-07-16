# `stage_run` terminal request control

- **状态**：Approved by the user's explicit request to fix Continue and validate it headlessly
- **范围**：`golish-agent-runtime` agent loop + `golish-agent-kit` blocked-stage event semantics；no schema/migration/IPC/frontend change
- **现场**：session `pentest-chat-1784179823492-1`, operation `a8029de1-9f37-4450-b7e9-f08f7ba4c371`

## Problem

`stage_run` correctly returned the durable `STAGE_TEAM_OPERATOR_RECOVERY_REQUIRED` state for the
outcome-unknown `eas_fingerprint_services` child. The result intentionally used
`success=true, passed=false`: reading a durable blocker succeeded, so ordinary failed-tool fallback
must not run.

The primary agent loop only has a deterministic terminal signal for an accepted
`submit_stage_deliverable`. It therefore appended the blocked `stage_run` ToolResult and asked the
model for another completion. The model then attempted coverage reads, plan edits, direct work,
submission and another `stage_run`. Tool guards prevented unsafe execution, but the request wandered
instead of ending at the authoritative recovery barrier.

## Contract

1. A server-authored `stage_run` result may carry a closed `runtime_control` object:

   ```json
   {
     "kind": "halt_current_request",
     "reason": "operator_recovery_required"
   }
   ```

2. The dispatch layer translates only the exact `stage_run` + closed control tuple into a typed
   `ToolDispatchHaltReason`. Arbitrary tools, prose, `success=false`, or look-alike business fields
   cannot terminate a request.
3. Once the typed halt is observed, later calls in the same assistant tool batch receive paired
   synthetic ToolResults and are not executed. After the batch is recorded, the current agent loop
   ends without another provider completion.
4. The `stage_run` ToolResult remains `success=true`. The control signal is orthogonal to tool
   success and therefore cannot trigger capability fallback.
5. A same-request `stage_run` reentry block carries the same control kind with the closed reason
   `stage_run_reentry_blocked`; this makes the existing bounded retry guard terminal too.
6. A separate top-level user request still resets `StageRunReentryGuard` and resumes the same
   operation/Worker/message chain according to durable state. This change ends one request; it does
   not mark the task, stage, worker or active tool successful.
7. `run_stage_subtasks` emits `SubtaskCompleted` and appends `completed_results` only when the
   deterministic stage outcome has `gate_allowed=true`. A Gate BLOCK remains a resumable pause and
   must not render the frontend's “Step complete” marker.
8. The generic paused summary says the stage is incomplete and progress is preserved. It does not
   promise that another message alone can resolve every outcome-unknown/high-risk tool; automatic
   resume remains conditional on the closed recovery policy.

## Safety and observability

- No operator decision is synthesized and no outcome-unknown tool is replayed.
- The original ToolResult remains in history/transcript before termination.
- The runtime emits a trace log with the typed halt reason.
- The acceptance test uses a scripted completion model and injected headless `stage_run` executor.
  It proves the model is called once and that its scripted next-step coverage/direct/submit attempt
  is never reached. It uses no GUI, external provider or scan target.

## Non-goals

- Building the missing DB-backed operator-recovery UI.
- Automatically deciding retry/abandon/accept-known-outcome.
- Generalizing model-authored JSON into arbitrary loop control.
- Changing Gate, Stage Team recovery policy, or CLI/GUI operation authority.
