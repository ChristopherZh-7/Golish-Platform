# Golish Harness Observability Plane

- **Author**: Codex
- **Date**: 2026-05-26
- **Status**: Acknowledged (Phase 1 partial-satisfy via Evidence Ledger; full Observability Plane 推 Phase 2+)
- **Parent**: `docs/design/2026-05-26-operation-harness-profile-dag-lab.md` §21.9 Doc 4
- **Scope**: Phase 0 design only. No runtime code, no migration, no `resources/harness` config.

---

## 1. Purpose

Golish's harness needs more than evidence and gates. It also needs a first-class observability plane so operators and harness engineers can answer:

- What happened?
- Why did the agent make that decision?
- Which prompt, stage, tool, evidence, and gate result caused it?
- Can the operation be replayed?
- Did a harness change improve or regress behavior?

This document defines that observability layer.

---

## 2. One-Sentence Definition

> Evidence Ledger records security facts; Observability Plane records the process that produced, judged, replayed, and compared those facts.

In short:

```text
Evidence Ledger = what was found
Observability Plane = how it was found, why it was accepted or blocked, and how it changed between runs
Harness Lab = uses observability data to evaluate and improve the harness
```

---

## 3. Required Observability Surfaces

| Surface | Purpose | Example |
|---|---|---|
| Raw Event Log | Append-only record of every meaningful runtime event | LLM call started, tool call completed, gate blocked |
| Raw Artifact Store | Large raw outputs kept outside prompt context | nmap XML, Shodan JSON, HTTP response body |
| Trace Tree | Causal call tree across operation, stage, subtask, tool, evidence, gate | `operation_run -> stage_run -> subtask -> tool_call -> evidence -> gate` |
| State Timeline | Durable state transitions | `running -> blocked -> repair -> passed` |
| Metrics Rollup | Aggregated operational and evaluation metrics | cost, token count, pass rate, repair count |
| Operation Snapshot | Versioned snapshot of one complete run context | profile, scope, stage spec hash, charter hash, tool versions |
| Evaluation Record | Benchmark and gate outcomes | score, hard failures, failure taxonomy |
| Replay Record | Captures enough inputs to rerun or simulate a run | fixture ids, prompt hashes, tool response fixtures |
| Diff Record | Compares two runs | pass/fail flip, evidence diff, gate reason diff |
| Decision Attribution | Explains why a decision occurred | gate blocked because required DNS evidence missing |

---

## 4. Relationship To Existing Concepts

### 4.1 Evidence Ledger

Evidence Ledger should stay focused on facts:

```text
subject
evidence kind
observed value
scope label
raw artifact reference
as_of_timestamp
classification history
```

Observability Plane should record the surrounding process:

```text
which stage requested it
which subtask caused it
which tool produced it
which prompt and charter were active
which gate consumed it
which decision resulted from it
```

### 4.2 Audit Log

Existing `audit_log` is the nearest current substrate, but it should not become an unstructured dumping ground.

Recommended split:

```text
audit_log
  append-only high-level event ledger

raw_artifacts
  large raw outputs and replay fixtures

trace_edges
  causal parent/child relationships

metric_rollups
  derived aggregates, recomputable where possible
```

Phase 0 may describe these shapes, but must not create migrations.

### 4.3 Harness Lab

Harness Lab cannot work from final pass/fail alone. It needs observability records to explain:

- Did the agent fail because of a prompt gap?
- Did the tool contract return ambiguous output?
- Did the gate over-block?
- Did scope policy prevent an unsafe transition?
- Did the same benchmark regress after a charter change?

---

## 5. Canonical Trace Tree

Every important object should be linkable into one causal tree:

```text
operation_run_id
  operation_snapshot_id
  stage_run_id
    stage_spec_hash
    charter_hash
    sprint_contract_id
    subtask_id
      llm_call_id
      tool_call_id
        raw_artifact_id
        evidence_audit_id
        evidence_classification_id
    deliverable_id
    gate_result_id
      blocking_reason_id
      repair_subtask_id
```

The trace tree should answer:

```text
Which prompt led to this tool call?
Which tool call produced this evidence?
Which evidence supported this finding?
Which gate consumed this finding?
Which rule blocked or allowed the stage?
Which repair subtask was generated from the gate result?
```

---

## 6. Raw Event Model

Raw events should be append-only and minimal. They should carry stable ids and references, not giant payloads.

Candidate event kinds:

```text
operation_started
operation_completed
stage_started
stage_paused
stage_resumed
stage_passed
stage_failed
subtask_planned
subtask_started
subtask_completed
llm_call_started
llm_call_completed
tool_call_started
tool_call_completed
evidence_recorded
classification_recorded
deliverable_submitted
gate_started
gate_blocked
gate_passed
approval_requested
approval_granted
approval_denied
repair_subtask_created
replay_started
replay_completed
```

Minimal shape:

```json
{
  "event_id": "evt_...",
  "event_kind": "tool_call_completed",
  "operation_run_id": "op_...",
  "stage_run_id": "stage_...",
  "subtask_id": "subtask_...",
  "parent_event_id": "evt_...",
  "timestamp": "2026-05-26T12:00:00Z",
  "actor": "stage_executor",
  "refs": {
    "tool_call_id": "tool_...",
    "raw_artifact_id": "raw_...",
    "evidence_audit_id": 123
  }
}
```

---

## 7. Metrics Rollup

Metrics are derived views over raw events, traces, evidence, and gate results.

Operational metrics:

- stage duration
- tool call count
- tool error rate
- LLM token count
- LLM cost
- repair attempt count
- approval wait time
- abandoned run count

Harness quality metrics:

- gate pass rate
- hard fail rate
- fake completion block rate
- evidence coverage
- scope discipline
- replay determinism
- pass/fail flip rate after harness changes

Bench metrics:

- precision
- recall
- F1
- coverage
- evidence quality
- report usefulness

---

## 8. Operation Snapshot

Each operation run should have a versioned snapshot so later replay and attribution are meaningful.

Snapshot contents:

```text
profile id and version
scope rules version
stage spec hash
charter git hash
sprint contract id
tool wrapper versions
model/provider ids
temperature and decoding config
user intent constraints
evidence aging policy version
approval policy version
bench fixture ids, if any
```

Without this snapshot, a replay can accidentally run against a different harness and produce misleading comparisons.

---

## 9. Evaluation Record

Gate results and benchmark results should be persisted as evaluation records.

Candidate shape:

```json
{
  "evaluation_id": "eval_...",
  "operation_run_id": "op_...",
  "stage_run_id": "stage_...",
  "bench_case_id": "external_surface_basic_001",
  "status": "blocked",
  "hard_failures": ["MISSING_EVIDENCE"],
  "scores": {
    "coverage": 0.8,
    "evidence_quality": 0.6,
    "scope_discipline": 1.0
  },
  "failure_taxonomy": ["evidence_gap", "tool_contract_gap"],
  "gate_result_id": "gate_..."
}
```

---

## 10. Replay And Diff

Replay needs two modes:

| Mode | Meaning |
|---|---|
| Fixture Replay | Tool responses are replayed from saved artifacts |
| Live Replay | Same operation is rerun against a live lab environment |

Diff should compare:

- stage status
- gate status
- blocking reasons
- evidence ids and evidence kinds
- discovered assets
- skipped checks
- tool call sequence
- LLM cost and latency
- final report findings

The important output is not only "run A passed and run B failed". It is:

```text
Run B failed because gate rule MISSING_DNS_EVIDENCE blocked two findings that Run A accepted without evidence.
```

---

## 11. Decision Attribution

Decision attribution explains why the harness made a decision.

Examples:

```text
Why did the stage fail?
  Gate validate_external_attack_surface_gate returned MISSING_EVIDENCE for finding api.example.test.

Why was a target marked out of scope?
  ScopeService matched organizations.scope_rules_version=7 rule out[*.admin.example.test].

Why was a repair subtask created?
  Gate result gate_123 had required_repairs[0] = "Resolve DNS or mark skipped with reason."

Why did the replay regress?
  Charter hash changed from A to B; B removed the required DNS check instruction.
```

Attribution should be deterministic where possible. LLM-generated explanations can summarize attribution, but they should not be the source of truth.

---

## 12. Non-Goals For Phase 0

This document does not authorize:

- runtime implementation
- database migrations
- new Tauri commands
- new `resources/harness` config files
- UI dashboards
- live benchmark execution
- changing `task_orchestrator`

It only defines the observability model needed before implementing Harness Lab and replay.

---

## 13. Open Questions

1. Should raw artifacts live in DB, filesystem, or project export bundle first?
2. Should trace edges be explicit rows, or derived from `parent_id/run_id` plus ids in `detail`?
3. Which metrics must be materialized versus recomputed on demand?
4. How much prompt text is safe to store by default?
5. How should secrets inside raw tool output be redacted before replay?
6. Should replay be deterministic fixture-only for MVP, with live replay deferred?
7. What is the minimum UI needed: timeline, trace tree, metrics table, or all deferred?

---

## 14. Recommended First Phase

For the first harness MVP, observability should support only one stage:

```text
profile = assessment
authorization = L2 active_recon
stage = external_attack_surface
```

Minimum useful observability:

```text
raw event log
trace tree ids
stage state timeline
tool call to evidence linkage
gate result with blocking reasons
operation snapshot with profile/stage/charter hashes
```

Replay, diff, metrics rollups, and decision attribution can be designed now but implemented after the first stage gate is stable.
