# Runtime Monitor + Fine-Grained Resume

> Date: 2026-06-24
> Status: Design draft, no implementation in this document; updated to Mentor-first, rule-guarded supervision
> Related: `2026-06-03-background-tool-execution.md`, `2026-06-05-unified-ai-harness-observability.md`, `2026-06-12-unified-refiner.md`, `docs/superpowers/plans/2026-06-04-task-resume-after-disconnect.md`
> Invariants: AGENTS.md I7 evidence-first deliverables, I8 checked-empty != unchecked, §2.5 deterministic gate validator

Implementation note 2026-06-24: P1a/P1b mentor gating is wired as an env-controlled
runtime mode. `GOLISH_EXECUTION_MENTOR=shadow` invokes the existing execution
mentor and records advice to `harness::mentor` tracing only; `soft`/`on` appends
the advice to the tool response; default is `off`.

## 1. Problem

The current harness already has several useful pieces: background tool jobs, live output chunks, completion events, a state-driven task resume entry, and the unified gate Refiner. The remaining gap is that these pieces do not yet form one runtime supervision model.

Three user-visible failures motivate this design:

1. **Fast background failure can be missed by the model.** A tool with `background: true` may return "backgrounded" to the agent before a deterministic argument/runtime failure is visible in the same turn. The completion event arrives later, but the running sub-agent may already continue.
2. **Submit can become an invisible wait.** If `submit_stage_deliverable` waits for background scans to settle internally, the UI shows the submit call spinning rather than explicit "wait background jobs -> read output -> submit" steps.
3. **Resume is too coarse.** The project has operation/stage graph resume, but interruption inside a stage often resumes by re-running that stage. It does not yet resume from "tool completed, evidence booked, next agent turn pending".

The design goal is to make runtime supervision explicit, observable, and resumable while allowing model supervision early. The model supervisor is a mentor/coach, not the deterministic judge.

## 2. Current Substrate

### 2.1 Existing stage graph resume

`execute_task_mode` already anchors a chat session to one DB session, checks `latest_resumable_by_session`, and calls `TaskOrchestrator::resume` when a checkpointed non-terminal task exists. The graph executor can `resume(thread_id, inject)` from `DbFlowCheckpointer`, which stores `operation_state.state_blob.graph_flow.{state,next_node}`.

This is good, but the checkpoint state is currently stage-flow shaped:

```text
OperationFlowState {
  seeded stage outcomes,
  visited stages,
  applied stage outcomes,
  next_node in graph_flow
}
```

That means it can resume the operation graph, not the inner agent/tool turn.

### 2.2 Existing background execution

The background execution design introduced a manager, job ids, completion events, and `check_job`-style observation. Recent work also added startup grace, live output chunks, sub-agent attribution, and visible wait tooling. These are the right primitives for runtime monitoring.

What is still missing is a single owner of policy:

- whether a background job failure must block the next agent turn,
- whether submit is allowed while jobs are running or unread,
- how a pending background job is represented in a resumable checkpoint,
- how repeated/no-progress tool patterns become corrections.

### 2.3 Existing Refiner

The unified Refiner is intentionally deterministic. It runs after a gate BLOCK and returns a correction plus an optional submit-only lock. It should stay gate-level and should not become a general runtime mentor.

This design keeps four roles separate:

```text
Hard rules: deterministic stop lines for fast failures, background jobs, submit preflight, evidence existence
RuntimeMonitor: watches tool/job/turn events before gate and owns operation-scoped runtime state
Mentor: model supervisor that reviews compact runtime context and proposes next-step advice
Refiner: deterministic gate BLOCK repair after final validation fails
```

The Mentor may run earlier than P2, but it does not decide PASS/BLOCK and does not replace hard rules.

## 3. Design Principles

1. **Rule-guarded model supervision.** The Mentor can advise early, but fast failures, unread background output, pending jobs, submit preflight, and evidence existence remain rule-driven.
2. **Soft supervision by default.** The monitor should normally inject the next correction or block the next unsafe action, not kill a running sub-agent.
3. **No hidden waits.** If the system needs to wait for jobs, it should appear as an explicit tool step or harness event.
4. **Resume from facts, not prose.** Checkpoints must record tool/job/evidence facts so recovery can skip work that already completed.
5. **Never bypass the gate.** RuntimeMonitor may stop bad flow earlier, but PASS/BLOCK remains the deterministic gate's job.
6. **Evidence is the commit point.** A tool run is reusable after resume only when its output is captured and, when relevant, evidence/storage hooks have completed.

## 4. Architecture

```text
LLM turn
  -> tool call
  -> ToolRunner
      -> ToolEvent stream
          started / stdout / stderr / exit
          background_started / background_output / background_completed
          evidence_booked / storage_updated
  -> RuntimeMonitor
      -> RuntimeCorrection queue
      -> BackgroundJobSnapshot registry
      -> AgentRunCheckpoint patch
      -> optional Mentor review on trigger
          -> MentorAdvice
          -> RuntimeCorrection source=mentor
  -> Agent loop
      -> inject correction before next LLM turn
      -> or force explicit wait/read tool before submit
  -> submit_stage_deliverable
      -> submit preflight reads RuntimeMonitor state
  -> Gate
      -> PASS or BLOCK
  -> Refiner only on gate BLOCK
```

The monitor should live close to the existing agent execution boundary, not inside individual pentest tools. It consumes normalized events from `golish-app-core`/`golish-agent-app`/`golish-agent-runtime` and writes one operation-scoped runtime state.

## 5. P0: Runtime Preflight And Background Discipline

P0 is the immediate bug-prevention layer. It does not require true P2 checkpoint/resume.

### 5.1 Background startup grace

For AI-elected background calls, do a short startup confirmation window.

```text
background=true
  -> spawn once
  -> observe for startup_grace_ms
  -> if process exits: return final stdout/stderr/exit_code synchronously
  -> if deterministic startup error appears: return failure synchronously
  -> otherwise: return backgrounded job_id
```

Recommended default: 800ms to 2000ms. This catches bad flags, missing binaries, Ruby/runtime boot errors, permission errors, and immediate config mistakes without forcing long scans to run foreground.

### 5.2 Background job registry contract

Every background job must have an operation-scoped snapshot:

```rust
struct RuntimeJobSnapshot {
    job_id: String,
    operation_id: Option<Uuid>,
    chat_session_id: Option<String>,
    agent_path: String,
    stage: Option<StageKind>,
    tool_name: String,
    args_hash: String,
    command_preview: String,
    status: JobStatus, // Running | Succeeded | Failed | TimedOut | Cancelled | Abandoned
    exit_code: Option<i32>,
    stdout_tail_ref: Option<BlobRef>,
    stderr_tail_ref: Option<BlobRef>,
    evidence_ids: Vec<i64>,
    output_read_by_agent: bool,
    completed_at: Option<DateTime<Utc>>,
}
```

`output_read_by_agent` matters because a completed scan that the model never read should not be silently treated as handled.

### 5.3 Submit preflight

`submit_stage_deliverable` should not internally spin for long-running background jobs. Before accepting the submit, it asks RuntimeMonitor for the current stage's unsettled jobs.

Blocking cases:

- `Running`: the agent must call explicit wait.
- `Failed` and not acknowledged: the agent must inspect and either correct/re-run or mark blocked/not_applicable with a note.
- `Succeeded` but output unread and no evidence/storage hook consumed it: the agent must read/observe output first.
- `Abandoned`: after process restart, job existed in checkpoint but no live process/completion exists; the agent must reattach or re-run.

The correction is concrete:

```text
You still have background jobs for this stage. Call wait_for_background_jobs
for job_ids=[...] and then read/check their output before submitting.
Do not submit until each job is succeeded/failed/acknowledged.
```

### 5.4 Explicit tools/events

Prefer explicit visible steps:

- `wait_for_background_jobs({ job_ids?, stage?, timeout_secs? })`
- `check_job({ job_id, tail_bytes? })`
- optional future `ack_background_job({ job_id, disposition, note })`

If `wait_for_background_jobs` already exists in the current tree, this design makes it the required submit preflight recovery path, not an optional convenience.

## 6. P1: Rule-Guarded Mentor + RuntimeMonitor Corrections

P1 introduces operation-scoped runtime state and early model supervision. It is split into two safe modes:

- **P1a Mentor shadow mode:** the Mentor reviews triggered situations and writes advice into trace only. The agent does not see it yet.
- **P1b Mentor soft injection:** proven advice is injected as a RuntimeCorrection before the next LLM turn. Hard rules still decide what is blocked.

This makes model supervision useful before P2 resume work, without letting it become the judge.

### 6.1 RuntimeCorrection

```rust
enum RuntimeCorrectionSource {
    Rule,
    MentorShadow,
    MentorSoft,
}

enum RuntimeCorrectionKind {
    FastToolFailure,
    BackgroundStillRunning,
    BackgroundFailed,
    BackgroundOutputUnread,
    SubmitBeforeBackgroundSettled,
    RepeatedToolNoProgress,
    EvidenceNotBookedAfterToolSuccess,
    ToolRuntimeMisconfigured,
}

struct RuntimeCorrection {
    source: RuntimeCorrectionSource,
    kind: RuntimeCorrectionKind,
    operation_id: Uuid,
    stage: Option<StageKind>,
    agent_path: String,
    tool_name: Option<String>,
    job_ids: Vec<String>,
    evidence_ids: Vec<i64>,
    message: String,
    submit_allowed: bool,
    created_at: DateTime<Utc>,
}
```

Rule corrections may block unsafe actions, such as submit-before-background-settled. Mentor corrections are advice unless explicitly promoted to soft injection. Neither kind is a gate decision.

### 6.2 Deterministic trigger policy

The deterministic monitor decides when to trigger Mentor review. Triggers are cheap and explainable:

- tool failure with non-zero exit or known runtime error,
- background job completed or failed,
- submit returned `needs_fix`,
- same tool + materially same args N times without new evidence,
- many tool calls in same stage with no `evidence_booked`/storage update,
- repeated submit `needs_fix` with the same reason,
- repeated background failures for the same tool/runtime.

Thresholds should be configurable per stage/profile. The first implementation should use conservative defaults. Hard blocking is still reserved for deterministic safety cases.

### 6.3 Mentor context and output

The Mentor is a separate model-backed role, not the Refiner. It receives compact context:

```text
stage charter
recent tool calls/results
background job states
evidence delta since stage start
last gate/submission reason if any
active hard-rule blockers, if any
```

Mentor output must be structured:

```rust
struct MentorAdvice {
    class: MentorAdviceClass, // FixToolArgs | WaitBackground | ReadOutput | RerunDifferentTool | SubmitNow | StopAndReportBlocked | Generic
    confidence: MentorConfidence,
    rationale: String,
    suggested_next_step: String,
    may_inject: bool,
}
```

The Mentor must not create deliverables, cite invented evidence ids, or override gate decisions. In shadow mode, advice is written to trace only. In soft-injection mode, `may_inject=true` advice becomes a `RuntimeCorrection` shown to the agent.

### 6.4 Mentor-first, rule-guarded behavior

Examples:

| Situation | Hard rule | Mentor role |
|---|---|---|
| `naabu -ports` exits 2 quickly | return failure / block continuing as background success | suggest `-p` and rerun |
| background nmap still running | block submit | tell agent to wait/check job |
| background job failed | block or force inspection | explain likely cause and next tool |
| repeated `httpx` no output | no hard block unless submit too early | suggest alternative checks or mark checked_empty with evidence |
| gate BLOCK | gate blocks | Refiner handles repair, Mentor does not replace it |

## 7. P2: Fine-Grained Resume

P2 changes the resume unit from stage graph node to agent/tool turn. This should start only after P0/P1 are stable, because it depends on trustworthy ToolEvent and background job state.

### 7.1 New checkpoint layer

Keep the existing `graph_flow` checkpoint. Add a nested stage attempt checkpoint under `operation_state.state_blob`.

```json
{
  "graph_flow": {
    "state": {},
    "next_node": "external_attack_surface"
  },
  "agent_run": {
    "schema_v": 1,
    "operation_id": "...",
    "stage": "external_attack_surface",
    "stage_attempt_id": "...",
    "agent_path": "main>prober",
    "status": "waiting_next_turn",
    "llm_turn_index": 7,
    "message_chain_ref": "...",
    "pending_gate_correction": null,
    "pending_submit_only": false,
    "runtime_corrections": [],
    "background_job_ids": ["job_123"],
    "evidence_watermark": 2451,
    "last_tool": {
      "tool_call_id": "...",
      "tool_name": "pentest_run",
      "state": "completed",
      "result_ref": "..."
    }
  }
}
```

This is not a replacement for transcripts. It is the minimum machine-readable state needed to continue safely.

### 7.2 Checkpoint boundaries

Checkpoint at stable boundaries only:

1. before LLM turn,
2. after LLM response parsed,
3. before tool dispatch,
4. after tool started,
5. after tool completed or backgrounded,
6. after evidence/storage hook completed,
7. after runtime correction queued,
8. before submit,
9. after gate decision.

Do not checkpoint in the middle of streaming text. Streaming remains transcript/log data, not resume state.

### 7.3 Resume decision matrix

On resume, inspect `agent_run.last_tool` and background job snapshots:

| Last state | Resume action |
|---|---|
| `before_llm_turn` | Re-enter same LLM turn with runtime corrections + recovered context |
| `after_llm_response_parsed` but before tool | Dispatch parsed tool call if idempotency key was not used |
| `tool_started` and process still live | Reattach/listen; do not re-run |
| `tool_started` and process gone, no completion | Mark `Abandoned`; require re-run or manual ack |
| `tool_completed`, evidence/storage complete | Do not re-run; inject result/evidence refs and continue |
| `backgrounded`, job running | Continue with visible wait/check path |
| `backgrounded`, job completed | Inject completion/result before next LLM turn |
| `submit_in_flight` | Re-run submit only if deliverable idempotency key says no accepted result exists |
| `gate_blocked` | Re-inject pending Refiner correction |

### 7.4 Idempotency

P2 needs idempotency keys for operations that can otherwise duplicate side effects:

```text
operation_id + stage_attempt_id + agent_path + tool_call_id
```

Use this key for:

- evidence booking dedupe,
- background job snapshot association,
- stage deliverable submit dedupe,
- output-store structured landing dedupe where possible.

No side-effecting tool should be automatically re-run on resume unless the previous call is proven not to have started or is explicitly marked abandoned.

### 7.5 Persisting bulky outputs

Do not put full stdout/stderr into `operation_state.state_blob`. Store only refs/tails:

```rust
enum BlobRef {
    TranscriptEvent { session: String, event_id: String },
    BackgroundJob { job_id: String },
    ArtifactPath { path: String },
    AuditLogEvidence { evidence_id: i64 },
}
```

The checkpoint should stay small and durable; transcripts/run.log/artifacts hold the large data.

## 8. UI/Trace Semantics

The UI should show runtime supervision as explicit activity:

```text
pentest_run nmap ...        backgrounded, job_123
wait_for_background_jobs    waiting for 3 jobs
check_job job_123           failed: bad flag
Runtime correction          fix args, use -p not -ports
pentest_run naabu ...       succeeded
submit_stage_deliverable    accepted or needs_fix
```

Avoid submit calls that visually spin while doing hidden waiting. A long wait belongs to a wait/check job step.

Every monitor action should be written to transcript/run.log as a structured trace event:

- `RuntimeMonitorCorrectionQueued`
- `MentorAdviceRecorded`
- `MentorAdviceInjected`
- `RuntimeMonitorSubmitPreflightBlocked`
- `BackgroundJobObserved`
- `AgentRunCheckpointSaved`
- `AgentRunResumeDecision`

## 9. Where This Should Live

Suggested ownership:

| Concern | Crate/module |
|---|---|
| raw job process state | `golish-app-core::background_jobs` |
| normalized tool/job events | `golish-core` events + `golish-agent-app` bridge |
| RuntimeMonitor policy | `golish-agent-kit::task_orchestrator` or a new `runtime_monitor` submodule |
| Mentor prompt/chain | `golish-agent-kit` or `golish-sub-agents` pipeline-only role, but invoked by RuntimeMonitor |
| agent run checkpoint structs | `golish-agent-kit` pure types, persisted by DB shim |
| DB query/write helpers | `golish-db` repo / existing `operation_state` JSON blob |
| UI event rendering | frontend ai-events + tool/sub-agent detail views |

Do not put monitor policy inside pentest-specific tools. The same semantics should apply to `run_pty_cmd`, `pentest_run`, and future long-running tools.

## 10. Rollout

### Phase 0: Design and trace audit

- Confirm existing startup grace, wait tool, completion event, live output, and submit preflight behavior.
- Add/refresh run_tree output for runtime monitor events if needed.

### Phase 1: P0 hardening

- Make submit preflight always explicit.
- Ensure fast failure sync return works for `background: true`.
- Ensure failed background jobs are injected into the same sub-agent context when possible.

### Phase 2: P1a RuntimeMonitor state + Mentor shadow mode

- Add operation-scoped runtime state and deterministic corrections.
- Add tests for background failed/unread/running submit preflight.
- Add Mentor trigger points and compact context builder.
- Record `MentorAdviceRecorded` in trace only; do not inject to the agent.

### Phase 3: P1b Mentor soft injection

- Promote high-confidence shadow advice into soft RuntimeCorrections.
- Keep submit/background/evidence blockers deterministic.
- Add regression runs comparing shadow advice against real gate/submission outcomes.

### Phase 4: P2a checkpoint schema

- Persist `agent_run` checkpoint at stable boundaries.
- Resume read-only/safe states first: completed tool, backgrounded running/completed, pending correction.
- Do not auto-rerun side-effecting tools yet.

### Phase 5: P2b reattach and idempotency

- Reattach live background processes after app/process restart where possible.
- Add idempotency keys around submit/evidence booking/tool results.
- Support `Abandoned` classification and guided re-run.

## 11. Open Decisions

1. **Persistence form:** keep `agent_run` inside `operation_state.state_blob`, or create a new `agent_run_checkpoints` table? Recommendation: start in `state_blob` for P2a, move to table only when querying/GC becomes painful.
2. **Ack semantics:** should failed background jobs require explicit `ack_background_job`, or is reading output enough? Recommendation: reading output is enough for P0/P1; explicit ack later if gate needs audit semantics.
3. **Sub-agent interruption:** should monitor ever cancel a sub-agent? Recommendation: no for P0/P1. Hard cancel only for deterministic runaway/hard authorization problems.
4. **Mentor model:** reuse default model or a configured stronger adviser model? Recommendation: support a separate configured mentor model, but allow fallback to current provider/model for shadow mode.
5. **Mentor promotion:** what advice can move from shadow to soft injection? Recommendation: start with safe classes only: `FixToolArgs`, `WaitBackground`, `ReadOutput`, and `RerunDifferentTool`; keep `SubmitNow` shadow-only until validated.
6. **Tool idempotency scope:** shell commands cannot be generally idempotent. Recommendation: never auto-rerun a started shell command; only guide the agent after `Abandoned`.

## 12. Success Criteria

1. A bad background command returns its error in the same tool result if it fails during startup grace.
2. A long background scan is visible as a job, and submit is blocked with a clear wait/read instruction until it settles.
3. Background completion/failure becomes a RuntimeCorrection in the right agent context.
4. Mentor shadow advice is recorded for tool failures, background completion/failure, repeated no-progress, and submit `needs_fix`.
5. Mentor soft injection can advise next actions but cannot unblock submit, fabricate evidence, or mark a stage PASS.
6. A killed/restarted operation resumes from graph checkpoint and preserves pending background/job/correction facts.
7. P2a can resume after "tool completed + evidence booked" without re-running that tool.
8. No monitor or mentor path can mark a stage PASS; only the gate can.
