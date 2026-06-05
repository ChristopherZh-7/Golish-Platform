# Unified AI + Harness Observability (Self-Discoverable Trace)

- **Author**: MCP-agent-4 (Claude Opus 4.8)
- **Date**: 2026-06-05
- **Status**: Design — awaiting review before implementation
- **Parent vision**: [`2026-05-26-harness-observability-plane.md`](2026-05-26-harness-observability-plane.md) (Phase-0 vision: trace tree / replay / diff / metrics — still deferred). **This doc is the pragmatic first slice that realizes that vision's _Raw Event Log_ + _Trace Tree ids_ subset, scoped to one concrete goal: make a stuck run debuggable by an AI without the user pointing the way.**
- **Related**: `2026-06-03-harness-profile-driven-execution.md` (§P2-G added the gate-decision tracing chokepoint this doc builds on), `2026-06-03-background-tool-execution.md`, `2026-06-04-task-resume-after-disconnect.md`

---

## 1. Why this doc exists (the concrete pain)

Over the last several sessions, debugging "why is the agent stuck at the `target_intel` gate?" required:

- Grepping **~88k lines** of `~/.golish/backend.log` to reconstruct one operation's decision timeline.
- Manually cross-referencing **three disconnected worlds**: the main-agent transcript (`transcript.json`), the per-sub-agent transcripts (`subagents/<id>/transcript.json`), and the harness decisions that live **only** in `backend.log` tracing.
- The user repeatedly **pointing the way** ("look at the AI log AND the backend log", "it's stuck adding the knowledge graph") because the agent could not locate the relevant signal itself.

The root issue is not any single bug — it is that **the system has no unified, self-discoverable trace**. The two explicit requirements from the user:

1. **Self-discoverability** — an AI (the in-product agent, or a debugging agent like Cursor) should be able to find the relevant logs **itself**, given only a session/operation handle, instead of the human pointing at files.
2. **Agent + sub-agent correlation** — agent and sub-agent activity must be displayed/stored in a form that **threads together** into one traceable timeline.

This doc designs that. It deliberately does **not** attempt the full replay/diff/metrics plane from the 2026-05-26 doc — those stay deferred. The goal here is: **given an `operation_id`, an AI can reconstruct the complete cross-agent decision timeline in one tool call or one file read, and immediately see _why_ a stage passed or blocked.**

---

## 2. Current state (evidence-backed map)

All claims below were verified by reading the source at the cited `path:line`. This is the substrate the design must work with.

### 2.1 Where AI activity is logged today (3 disconnected sinks)

| Sink | What lands there | Path / location |
|---|---|---|
| **Main transcript** | Non-streaming `AiEvent`s for the main agent | `{base}/{session_id}/transcript.json` (JSONL despite the `.json` name) — `golish-events/src/transcript/mod.rs:83-85` |
| **Sub-agent transcripts** | Only `SubAgentToolRequest` + `SubAgentToolResult`, one file **per sub-agent** | `{base}/{session_id}/subagents/{agent_id}-{parent_request_id}/transcript.json` — `golish-sub-agents/src/transcript.rs:23,59-69` |
| **backend.log** | All harness decisions (gate, evidence, submit, note injection) via `tracing` | `~/.golish/backend.log` — `golish/src/telemetry/init.rs:73-83` |

`base` resolution: `VT_TRANSCRIPT_DIR` env → else `{workspace}/.golish/transcripts` → else `~/.golish/transcripts` (`golish-agent-app/.../session.rs:95-108`).

### 2.2 `should_transcript` filters out streaming + sub-agent internals

```23:34:backend/crates/golish-events/src/transcript/mod.rs
pub fn should_transcript(event: &AiEvent) -> bool {
    !matches!(
        event,
        AiEvent::TextDelta { .. }
            | AiEvent::Reasoning { .. }
            | AiEvent::ToolOutputChunk { .. }
            | AiEvent::SubAgentToolRequest { .. }
            | AiEvent::SubAgentToolResult { .. }
            | AiEvent::SubAgentTextDelta { .. }
            | AiEvent::SubAgentReasoning { .. }
    )
}
```

So the main transcript already excludes the noisiest streaming. But sub-agent **tool** events are excluded here yet written to the **separate** sub-agent files — i.e. the split is by file, not merged.

### 2.3 Harness decisions are `tracing`-only (the killer gap)

| Decision | Emitted as `AiEvent` (→ transcript/UI)? | `tracing` (→ backend.log only)? | Evidence |
|---|---|---|---|
| Gate **PASS** | **Yes** — reuses `TaskProgress{status:"stage_passed"}` | Yes | `execute.rs:365-388` |
| Gate **BLOCK** | **No event at all** | Yes (`harness::hook`) | `execute.rs:365-372`, `931-936` |
| Per-check pass/block (schema/contract/vacuous/freshness/scope) | **No** | Yes (`harness::gate::*`) | `harness/gate/*.rs` |
| Evidence row booked (`log_evidence`) | **No** (DB insert is silent) | Partly: sync append logs `harness::evidence`; DB insert itself logs nothing | `golish-db/.../audit/mod.rs:174-207`, `direct/mod.rs:291-296`, `bridge_config.rs:200-205` |
| `submit_stage_deliverable` accepted / needs_fix | **No** (only the tool's return JSON, surfaced as a generic `ToolResult`) | No (only an infra `warn!`) | `harness_submit_tool.rs:204-293,129-133` |
| Background note injection | **No** | Yes (`prepare.rs:55-57`) | `golish-agent-bridge/.../prepare.rs:43-67` |

The code itself documents the deferral — `consume_gate_outcome` reuses `TaskProgress` deliberately:

```378:388:backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs
            // Authoritative "stage passed its evidence gate" signal. ...
            // Reuses TaskProgress (message = stage id) to avoid a new AiEvent variant + a long exhaustive-match churn.
            self.emit(AiEvent::TaskProgress {
                task_id: task_id.to_string(),
                status: "stage_passed".to_string(),
                message: outcome.gated_stage.as_str().to_string(),
            });
```

**Consequence**: the single most useful debugging line — "deliverable cited `[1,2,3]` but the real ledger ids were `[86,88,90]`, so the gate blocked it as fabricated" — exists only as two separate `tracing::warn!` lines in an 88k-line file, never as a structured event a tool can return.

### 2.4 Four parallel "session" ids — no shared correlation key

| Id | Type | Created | Used by |
|---|---|---|---|
| chat session string (`event_session_id`, e.g. `pentest-chat-…`) | `String` | frontend IPC arg | transcript path, `RuntimeEvent.session_id`, span `langfuse.session.id` (**stripped from file output**, `telemetry/filter.rs:15-18`), `audit_log.session_id` (evidence join key) |
| orchestrator `session_id` (`sessions.id`) | `Uuid` | `sessions::upsert_by_chat_key` (`chat.rs:132-146`) | `tasks`/`subtasks` FK |
| `task_id` == harness `operation_id` | `Uuid` | `tasks::create` (`orchestrator.rs:106-115`) | `operation_state`, `stage_runs`, gate logs (`harness::hook` `task_id=`) |
| `DbTracker.session_uuid` | `Uuid` (**random per bridge**) | `set_db_backend` (`config.rs:44-51`) | `sub_agent_dispatches`, evidence `operation_id` fallback |

**No single id is threaded through both the transcript AND backend.log end-to-end** (verified). `turn_id` is only on `AiEvent::Started`; harness logs carry `task_id` but transcript events carry the chat string + `task_id` as a `String` on a subset of variants; spans carry `langfuse.session.id` which the file layer strips. So you cannot `grep <one id>` across the AI log and the backend log.

Worse: evidence rows' `operation_id` in the hash chain is usually `DbTracker.session_uuid()` (random), **not** `task.id`, because `set_task_context` has **zero callers** (`direct/mod.rs:262-265`, confirmed in `agent-progress.md`). So even within the DB, evidence is not keyed by the harness operation.

### 2.5 Sub-agent ↔ parent linkage is half-wired

- Event protocol carries `parent_request_id` + `depth` on every `SubAgent*` variant (`event.rs:182-240`), and `ToolSource::SubAgent{agent_id, agent_name}` on main-agent tool events (`tool_source.rs:14-17`).
- DB `sub_agent_dispatches.parent_dispatch_id` column **exists** but the runtime passes `None` (`sub_agent_call.rs:160-168`) — the parent tree is not recorded.
- There is **no `agent_path`** anywhere (e.g. `main>pentester>reporter`). Reconstructing "which agent did this" requires joining `agent_id`+`parent_request_id`+`depth` by hand across files.

### 2.6 Multiple emit paths, no single choke-point

Events reach disk/UI through at least 5 paths (coordinator, legacy bridge, agentic-loop direct append, `event_tx` listener, raw stream deltas) — `coordinator.rs:303-326`, `agentic_loop/context.rs:230-272`, `agent_bridge/events.rs:161-187`, `stream_processor/chunks.rs:85-88`. This is why `SubAgentStarted`/`SubAgentCompleted` (which `should_transcript` would keep) can still fail to land in the main transcript: they go out on `event_tx` only. Any "stamp a correlation id on every event" plan must account for this fan-out.

### 2.7 Logging hygiene

- `tracing` init: `~/.golish/backend.log`, default level from `settings.advanced.log_level` (default `info`), plus a `harness={level}` directive for desktop (`bootstrap.rs:83-89`, `init.rs:38-160`). `RUST_LOG` is honored.
- **No `#[instrument]` / no `.in_scope` anywhere** in `backend/`. Harness uses target-based `info!`/`warn!` (`harness::hook`, `harness::evidence`, `harness::eval`) with manual fields. Spans that do exist (`turn/executor.rs`, `completion.rs`) attach `langfuse.*` fields that the **file layer strips**, so they don't help file-based debugging.
- A "decisions on, tokens off" view is **already achievable** with `golish=info,harness=debug` — but it's undocumented and the result still lives only in `backend.log`, disconnected from the transcript.

---

## 3. Goals / non-goals

### Goals

- **G1 (self-discovery)**: Given only an `operation_id` (or a chat session string), an AI can locate and read the complete decision timeline of that run **without the user**. One predictable path + one manifest + one tool.
- **G2 (agent+subagent as one timeline)**: Main agent and all sub-agents appear in **one merged, ordered timeline**, each line tagged with an `agent_path` (`main`, `main>pentester`, …) so causality is readable top-to-bottom.
- **G3 (decisions are first-class)**: Gate PASS/BLOCK (with fabricated ids / blocking reason), evidence booked (with id + source), submit accepted/needs_fix (with cited refs vs available real ids), and background-note injection become **structured records**, not buried `tracing` lines.
- **G4 (one correlation spine)**: A single `operation_id` is stamped on every record on both the AI side and the backend.log side, so `grep <operation_id>` works across all sinks.
- **G5 (signal over noise)**: A documented "harness debug" logging profile and a decisions-only default for the trace sink, so the useful 1% isn't drowned by token deltas.

### Non-goals (explicitly deferred to the 2026-05-26 plane)

- Replay (fixture or live), run-to-run diff, metrics rollups, evaluation records, operation snapshots, a UI dashboard, a new normalized DB schema with `trace_edges`/`raw_artifacts` tables. We reuse the **existing JSONL transcript substrate** for the first slice and design (but do not build) the DB substrate.

---

## 4. The design — five components

```text
┌─────────────────────────────────────────────────────────────────┐
│ A. Correlation spine: operation_id + agent_path stamped on        │
│    every AiEvent, every harness::* tracing line, evidence rows.    │
├─────────────────────────────────────────────────────────────────┤
│ B. Harness decisions as first-class events                        │
│    AiEvent::HarnessTrace { kind, operation_id, stage, agent_path, …}│
│    emitted at the existing chokepoints (gate/evidence/submit/note).│
├─────────────────────────────────────────────────────────────────┤
│ C. Unified, self-discoverable sink                                │
│    .golish/transcripts/<operation_id>/                            │
│      manifest.json   ← the single entry point an AI reads first    │
│      timeline.jsonl  ← merged main+subagent+harness, ordered       │
├─────────────────────────────────────────────────────────────────┤
│ D. Self-service retrieval                                         │
│    - agent tool `harness_trace(operation_id?, last_n?, kinds?)`    │
│    - CLI `just replay <operation_id|session>`                      │
├─────────────────────────────────────────────────────────────────┤
│ E. Logging hygiene                                                │
│    - documented `golish=info,harness=debug` profile               │
│    - stamp operation_id + agent_path on harness::* lines           │
└─────────────────────────────────────────────────────────────────┘
```

### 4.A Correlation spine: `operation_id` + `agent_path`

**Decision: adopt `operation_id` (= `task.id` Uuid) as the primary correlation key.** It already equals the harness operation, already keys `operation_state`/`stage_runs`, and is the natural unit of "one run". The chat session string remains a secondary lookup key (for resume + evidence join), recorded in the manifest.

Two new pieces of context threaded through the system:

1. **`operation_id: Uuid`** — stamped on:
   - Every transcript record (see §4.C — carried by the envelope/record wrapper, **not** by adding a field to all 51 `AiEvent` variants).
   - Every `harness::*` `tracing` line that currently has `task_id` (already mostly true — formalize it).
   - Evidence rows: fix `set_task_context` to be called so `operation_id` in the evidence chain = `task.id` (closes §2.4's silent divergence). This also makes `recent_evidence_ids` lookups keyable by operation.

2. **`agent_path: String`** — a human/grep-friendly lineage string built from the sub-agent chain:
   - `main` for the top-level agent.
   - `main>pentester` for a depth-1 sub-agent named `pentester`.
   - `main>pentester>reporter` for nested.
   - Derived deterministically from `SubAgentContext.parent_agent` + `agent_id`/`agent_name` + `depth` (`golish-sub-agents/src/definition/mod.rs:29-58`). Also wire `parent_dispatch_id` (column already exists) so the DB tree matches.

**Why `agent_path` over raw ids**: a debugging AI reading a merged timeline can group/filter by a readable prefix (`main>pentester`) instead of resolving opaque uuid pairs. This is the single highest-leverage change for G2.

### 4.B Harness decisions as first-class events

Add **one** new event variant (mitigating the "exhaustive-match churn" the code worried about by making it a single struct-carrying variant with a `kind` enum, so new decision kinds don't add match arms):

```rust
// golish-core/src/events/harness_trace.rs (new)
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum HarnessTraceKind {
    GateDecision {
        gate: String,          // "PASS" | "BLOCK"
        findings: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fabricated_evidence_refs: Vec<i64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        available_real_ids: Vec<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        first_blocking_reason: Option<String>,
    },
    EvidenceBooked { tool: String, evidence_id: i64, source: String }, // source: "sync" | "background"
    DeliverableSubmitted {
        status: String,        // "accepted" | "needs_fix" | "rejected"
        cited_evidence_refs: Vec<i64>,
        available_real_ids: Vec<i64>,
    },
    BackgroundNotesInjected { count: u32, evidence_ids: Vec<i64> },
}
```

And a single new `AiEvent` arm:

```rust
// added to AiEvent (event.rs)
HarnessTrace {
    operation_id: String,
    stage: String,
    #[serde(default)]
    agent_path: String,
    #[serde(flatten)]
    trace: HarnessTraceKind,
},
```

Emit points (reuse the **existing** chokepoints — additive, no behaviour change):

| Event | Where to emit | Already has a tracing line at |
|---|---|---|
| `GateDecision` (PASS **and** BLOCK) | `consume_gate_outcome` | `execute.rs:365-372` |
| `GateDecision.fabricated/available` | `enforce_evidence_existence` BLOCK path | `execute.rs:931-936` |
| `EvidenceBooked` | sync append + background listener | `direct/mod.rs:291-296`, `bridge_config.rs:200-205` |
| `DeliverableSubmitted` | `submit_stage_deliverable` return | `harness_submit_tool.rs:204-293` |
| `BackgroundNotesInjected` | `append_background_notes` | `prepare.rs:55-57` |

`should_transcript` returns `true` for `HarnessTrace` (it's a decision, never streaming), so it lands in the trace automatically.

### 4.C Unified, self-discoverable sink

Introduce an **operation-scoped** trace directory keyed by `operation_id` (stable, derivable, unlike the chat string):

```text
.golish/transcripts/op-<operation_id>/
├── manifest.json     ← the entry point: status, ids, stage list, agent paths, file map, last decision
└── timeline.jsonl    ← merged, time-ordered records (main agent + every sub-agent + harness decisions),
                         each line: { ts, seq, agent_path, operation_id, stage?, event }
```

- **Keep** the existing per-session `transcript.json` and per-sub-agent files for backward compatibility (other code + the frontend read them). The new merged `timeline.jsonl` is an **additional** write at the single coordinator choke-point (§4.A must first route all events — including sub-agent ones — through one place, or the merger tails the existing files).
- **`manifest.json`** is the heart of self-discovery. Example:

```json
{
  "operation_id": "0f1c…",
  "chat_session": "pentest-chat-abc",
  "title": "recon example.com",
  "status": "blocked",
  "current_stage": "target_intel",
  "stages": ["scoping", "target_intel"],
  "agent_paths": ["main", "main>pentester", "main>pentester>reporter"],
  "last_decision": { "kind": "gate_decision", "gate": "BLOCK", "stage": "target_intel",
                     "fabricated_evidence_refs": [1,2,3], "available_real_ids": [86,88,90] },
  "files": { "timeline": "timeline.jsonl", "backend_log": "~/.golish/backend.log" },
  "updated_at": "2026-06-05T00:00:00Z"
}
```

An AI debugging a stuck run reads **one file** (`manifest.json`) and immediately knows: the op blocked at `target_intel` because the deliverable cited fabricated ids while real ids existed — the exact conclusion that previously took an 88k-line grep.

### 4.D Self-service retrieval (the "don't make the user point the way" piece)

Two surfaces, same data:

1. **Agent tool `harness_trace`** (for the in-product agent to introspect its own run, and for any debugging agent):
   - Signature: `harness_trace(operation_id?: string, last_n?: number = 50, kinds?: string[])`.
   - Default `operation_id` = the current operation (from bridge/orchestrator context), so the agent can call it with no args.
   - Returns the merged decision timeline (filtered to harness/tool/decision kinds by default — **not** token deltas), newest-last, with `agent_path` on each line.
   - Complements the previously-proposed `list_evidence` tool (returns real ledger ids for the current op) — together they let a stuck agent self-correct ("I cited `[1,2,3]`; `harness_trace` shows the gate flagged them fabricated; `list_evidence` says real ids are `[86,88,90]`").

2. **CLI `just replay <operation_id|chat_session>`**:
   - Resolves the op dir, prints `manifest.json` summary, then the merged `timeline.jsonl`, optionally interleaving `backend.log` lines matched by `operation_id` (now greppable thanks to §4.A).
   - Debugging becomes one command instead of multi-file archaeology. This is also how E2E verification of future harness fixes will be done ("read the timeline, assert `gate_decision` PASS").

### 4.E Logging hygiene

- **Document** the `golish=info,harness=debug` profile (already supported via the `harness=` directive — `init.rs`/`bootstrap.rs`) as the canonical "show decisions, hide token TRACE" recipe, surfaced in `docs/development.md` and the manifest's `files` hint.
- **Stamp `operation_id` + `agent_path`** on every `harness::*` tracing line (most already have `task_id`; formalize and add `agent_path`). Fix the span fields that are currently stripped from file output, or stop relying on them for file debugging.
- Net effect: `rg <operation_id> ~/.golish/backend.log` returns the full cross-agent decision trail, and it lines up 1:1 with `timeline.jsonl`.

---

## 5. Key decisions (review these before executing the plan)

| # | Decision | Options | Recommendation |
|---|---|---|---|
| D1 | Primary correlation key | (a) chat session string, (b) `task.id`/`operation_id` Uuid | **(b)** — it is the harness operation unit and already keys gate/stage/evidence-state. Record chat string in manifest for the resume/evidence join. |
| D2 | One event variant vs many | (a) one `HarnessTrace{kind}` arm, (b) one arm per decision | **(a)** — single match arm avoids the "exhaustive-match churn" the code explicitly avoided (`execute.rs:382`); new kinds extend the inner enum only. |
| D3 | Trace storage | (a) JSONL files (reuse substrate), (b) new DB tables (`raw_events`/`trace_edges`) | **(a) first**; design the DB shape per 2026-05-26 §4.2 but defer it. JSONL is replayable, greppable, zero-migration. |
| D4 | Merge strategy for agent+subagent | (a) route all events through one choke-point then write merged, (b) write merged at coordinator + tail/merge sub-agent files post-hoc | **(a)** is cleaner long-term but touches the 5 emit paths (§2.6); **(b)** is lower-risk for the first slice. Recommend **(b) for Phase 1, (a) as Phase 2 cleanup.** |
| D5 | Stamp id on every `AiEvent` variant? | (a) add `operation_id` to all 51 variants, (b) carry it on the record/envelope wrapper | **(b)** — adding a field to 51 variants is churny and breaks ts-rs consumers; the wrapper (`TimestampedEntry` / a new `TraceRecord`) carries `operation_id` + `agent_path`. |
| D6 | Evidence `operation_id` fix | (a) wire `set_task_context` so it = `task.id`, (b) leave random `DbTracker` uuid | **(a)** — closes §2.4 divergence; makes evidence rows keyable by operation. Needs care (it changes the evidence hash-chain `operation_id`); gate by a test. |

---

## 6. Data model summary

- **`HarnessTraceKind`** (new, `golish-core/src/events/harness_trace.rs`) — see §4.B.
- **`AiEvent::HarnessTrace`** (new single arm) — see §4.B.
- **`TraceRecord`** (new wrapper, the merged-timeline line):
  ```jsonc
  { "ts": "…", "seq": 1234, "operation_id": "…", "agent_path": "main>pentester",
    "stage": "target_intel", "event": { /* AiEvent, internally tagged */ } }
  ```
- **`OperationManifest`** (new, `manifest.json`) — see §4.C; derives `Serialize/Deserialize` + `ts_rs::TS` so the frontend can render an op picker later.
- **`agent_path`** format: `main` then `>`-joined `agent_name` (fallback `agent_id`) per depth.

No DB migration in the first slice (D3/D6 caveat: D6 touches evidence-write call args, not schema).

---

## 7. Self-discovery protocol (how an AI debugs a stuck run, no user)

This is the acceptance scenario for G1/G2:

1. AI is told "the run is stuck" (or detects `status:"blocked"`). It has the chat session string or `operation_id`.
2. AI calls `harness_trace()` (no args → current op) **or** reads `.golish/transcripts/op-<operation_id>/manifest.json`.
3. `manifest.last_decision` shows `gate_decision BLOCK @ target_intel, fabricated=[1,2,3], available_real_ids=[86,88,90]`.
4. AI reads `timeline.jsonl` (or `harness_trace(kinds=["gate_decision","deliverable_submitted","evidence_booked"])`) and sees, in order, on a single `agent_path=main>pentester>reporter` line:
   `deliverable_submitted{cited:[1,2,3]}` → `gate_decision{BLOCK, fabricated:[1,2,3]}`, with `evidence_booked{86,88,90}` earlier on `main>pentester`.
5. Conclusion in seconds: the reporter sub-agent cited placeholders; real ids existed on its parent — a propagation problem — **without** grepping backend.log and **without** the user pointing.

If deeper detail is needed, `rg op-<operation_id> ~/.golish/backend.log` (now stamped) gives the per-check gate reasons.

---

## 8. Phasing & relationship to the 2026-05-26 plane

| Phase | Scope | Realizes (from 2026-05-26) |
|---|---|---|
| **P1 (this slice)** | A (spine: operation_id + agent_path) · B (HarnessTrace events) · C-(b) (manifest + merged timeline via post-hoc merge) · D (harness_trace tool + just replay) · E (hygiene) | Raw Event Log + Trace Tree ids + Decision Attribution (partial) |
| **P2** | C-(a) single choke-point refactor · D6 evidence operation_id wiring fully verified · `list_evidence` tool | State Timeline, cleaner causal tree |
| **P3 (deferred)** | DB substrate (`raw_events`/`trace_edges`/`raw_artifacts`), replay/diff, metrics rollups, evaluation records, UI | The remainder of the 2026-05-26 vision |

This doc does **not** mark 2026-05-26 superseded; it is the long-term vision and this is its first concrete installment.

---

## 9. Risks & rollback

- **R1 — emit fan-out (§2.6)**: stamping/merging must not miss events that bypass the coordinator. Mitigation: P1 uses post-hoc merge of existing files (low risk); P2 consolidates paths behind tests.
- **R2 — D6 evidence chain change**: altering the evidence `operation_id` source changes the hash chain. Mitigation: gate behind a unit test asserting chain validity + only change the `operation_id` arg, not the hashing.
- **R3 — ts-rs drift**: new `HarnessTrace`/`HarnessTraceKind`/`OperationManifest` must regenerate frontend types (`just check` enforces). Mitigation: include the binding regen + a frontend exhaustive-switch update in the same task.
- **R4 — volume**: `timeline.jsonl` per op is fine; `manifest.json` rewrite-on-update must be atomic (temp + rename). Mitigation: existing JSONL append for timeline, atomic write for manifest.
- **Rollback**: every piece is additive (new event arm defaulted, new files, new tool, new CLI subcommand). Reverting = remove the emit calls + the new files; existing transcripts/backend.log unaffected.

---

## 10. Open questions

1. Should `manifest.json` live under `op-<operation_id>/` (operation-keyed, recommended) **or** stay under the chat-session dir for one-hop discovery from the existing frontend? (Lean: op-keyed + a `chat_session→operation_id` index file.)
2. `harness_trace` default scope — current op only, or the whole chat session's latest op? (Lean: latest op of the session.)
3. Redaction: timeline may contain tool output with secrets. P1 inherits the existing transcript's (lack of) redaction; do we add a redaction pass now or defer with the rest of the replay plane? (Lean: defer, note the risk.)
4. Do we keep writing the legacy per-sub-agent files once the merged timeline exists, or deprecate them in P2? (Lean: keep through P1, revisit in P2.)
5. Should P1 also stamp `turn_id`/`iteration` into the timeline for sub-turn granularity, or is stage/agent_path enough for the debugging goal? (Lean: stage + agent_path for P1; add turn/iteration in P2 if needed.)

---

## 11. Definition of done (P1)

- `AiEvent::HarnessTrace` + `HarnessTraceKind` exist, derive ts-rs, frontend types regenerated, `should_transcript` keeps them.
- Gate PASS/BLOCK, evidence booked, submit outcome, and note injection each emit a `HarnessTrace` at their existing chokepoints (unit-tested).
- `.golish/transcripts/op-<operation_id>/{manifest.json,timeline.jsonl}` are produced for a run; `agent_path` is correct for nested sub-agents.
- `harness_trace` agent tool returns the merged decision timeline for the current op with no args.
- `just replay <operation_id|session>` prints manifest + merged timeline.
- `docs/development.md` documents the `golish=info,harness=debug` profile and the op-dir layout.
- `just precommit` green; evidence recorded in `agent-progress.md`.
