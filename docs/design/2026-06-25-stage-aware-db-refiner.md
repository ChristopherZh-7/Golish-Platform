# Stage-Aware DB-Backed Refiner

> Date: 2026-06-25
> Status: accepted design
> Related: `2026-06-12-unified-refiner.md`, `2026-06-24-runtime-monitor-and-fine-grained-resume.md`, `2026-06-24-eas-gate-contract.md`, `docs/superpowers/plans/2026-06-25-stage-aware-db-refiner.md`
> Invariants: AGENTS.md I7 evidence-backed stage delivery, I8 checked_empty != unchecked, deterministic gate remains final judge

## 1. Problem

Golish currently has several correction mechanisms that can fire in different places:

- deterministic gate `needs_fix`;
- task-level unified refiner;
- `stage_run` per-org retry;
- `SubmitRepairMode` and tool guards;
- runtime correction text from submit results;
- Execution Mentor / hard supervisor on repeated tool calls.

The latest `external_attack_surface` run showed the coordination problem. The gate eventually passed, but the worker needed several repair submissions:

```text
11 gaps -> 5 gaps -> 5 gaps -> 5 gaps -> 3 gaps -> 3 gaps -> accepted
```

The structured `coverage_gap_actions` helped the worker converge, but two weaknesses remained:

1. The repair loop lacked a stage-aware owner that can query DB truth and explain the exact repair strategy.
2. Execution Mentor can intervene during repair with generic repeated-tool advice, and that advice can conflict with the repair lock.

The desired behavior is:

```text
AI works -> submit_stage_deliverable -> deterministic gate needs_fix
  -> StageRefiner queries DB/evidence/stage/tool skills
  -> StageRefiner emits a structured RepairDirective
  -> ToolGuard enforces the directive
  -> AI executes only the repair
```

## 2. Goals

1. Make one stage-aware correction path responsible for "what should the agent do next?" after a gate or submit failure.
2. Allow each stage to have its own refiner logic, DB probes, and tool guidance.
3. Let the refiner read real state: evidence ledger, source/query logs, target tables, stage completions, background jobs, and stage specs.
4. Let the refiner read stage/tool "skills" before advising. In code terms, this means local deterministic instruction bundles: stage methodology, tool registry metadata, tool usage cards, and stage-specific repair policies.
5. Keep the gate deterministic. The refiner diagnoses and advises; it never decides PASS/BLOCK and never fabricates a deliverable.
6. Replace Mentor as a runtime intervention mechanism. Repeated-tool observation can remain telemetry, but it must not inject advice or block tools once StageRefiner exists.

## 3. Non-Goals

- No model-based PASS/BLOCK.
- No automatic deliverable synthesis by the refiner.
- No bypass of `submit_stage_deliverable`.
- No broad transcript stuffing into an LLM on every tool call.
- No new DB schema in the first slice.
- No full replacement of the existing task-plan refiner role; this design concerns gate/repair refiner behavior.

## 4. Terminology

| Term | Meaning |
|---|---|
| Gate | Deterministic validator for stage deliverables and stage closeout. |
| StageRefiner | New repair owner after gate/submit failure. Stage-aware, DB-backed, directive-producing. |
| RepairDirective | Structured instruction consumed by sub-agents and ToolGuard. |
| RefinerCore | Deterministic part: query state, compute gaps, classify root cause, render directive. |
| RefinerLLM | Optional model call used only for ambiguous or stalled repairs. |
| ToolSkill | Local tool guidance source: allowed usages, technique mapping, failure semantics, example commands. |
| Mentor | Existing repeated-tool LLM advisor. This design removes it from active intervention. |

## 5. Trigger Model

StageRefiner should trigger after failures, not during every normal tool call.

### 5.1 Primary triggers

1. `submit_stage_deliverable` returns `needs_fix`.
2. `stage_run` per-org gate returns BLOCK.
3. Outer task stage gate returns BLOCK.
4. Missing deliverable is detected for a stage.
5. Background job submit preflight returns unsettled/failed/unread jobs.

### 5.2 Escalation triggers

The deterministic RefinerCore runs on every primary trigger. RefinerLLM runs only when one of these is true:

- same gate reason hash repeats twice;
- `coverage_gap_actions` hash repeats twice;
- gap count does not decrease after a repair attempt;
- repair mode blocks forbidden tools more than once;
- evidence ids exist but status semantics remain ambiguous;
- stage policy has multiple valid terminal statuses and the worker chose the wrong one;
- final attempt before per-org retry exhaustion.

### 5.3 Mentor replacement

Execution Mentor should not inject advice or hard-block tools in the new design.

Recommended migration:

1. Disable Mentor active intervention in repair mode immediately.
2. Disable Mentor active intervention in `stage_run` specialists.
3. Keep repeated-tool detection as telemetry only if useful.
4. After StageRefiner is stable, remove Mentor prompt/LLM call/hard supervisor injection.

## 6. Architecture

```text
Agent / Specialist
  -> tool calls
  -> submit_stage_deliverable
      -> deterministic submit/gate checks
      -> needs_fix / BLOCK
          -> StageRefiner
              -> load StageSpec + methodology
              -> load ToolSkills for suggested tools
              -> query DB / evidence / source_query / jobs
              -> compute RepairDirective
              -> optional RefinerLLM on ambiguous/stalled repairs
          -> persist directive in agent_run checkpoint
          -> inject directive into next agent turn
          -> ToolGuard enforces directive
  -> resubmit
  -> deterministic gate PASS/BLOCK
```

This creates one owner for repair guidance while preserving hard boundaries:

- Gate decides validity.
- StageRefiner decides repair strategy.
- ToolGuard enforces allowed actions.
- StageRetry controls retry budget.

## 7. RepairDirective Contract

The directive must be structured first and text-rendered second.

```rust
struct RepairDirective {
    schema_v: u32,
    stage: String,
    org_id: Option<Uuid>,
    agent_path: String,
    repair_kind: RepairKind,
    root_cause: String,
    actions: Vec<RepairAction>,
    submit_guidance: SubmitGuidance,
    forbidden_tools: Vec<String>,
    allowed_tools: Vec<String>,
    evidence_ids: Vec<i64>,
    stale_or_unusable_evidence_ids: Vec<i64>,
    gate_reason_hash: String,
    gap_hash: Option<String>,
    llm_escalated: bool,
}

struct RepairAction {
    asset: Option<String>,
    technique: Option<String>,
    tool: Option<String>,
    command_hint: Option<String>,
    expected_status: Option<String>,
    evidence_refs: Vec<i64>,
    note: Option<String>,
    reason: String,
}

struct SubmitGuidance {
    mode: SubmitGuidanceMode,
    required_coverage_cells: Vec<CoverageCellDraft>,
    required_claims: Vec<ClaimDraft>,
    top_level_evidence_refs: Vec<i64>,
}
```

The directive is the only thing the sub-agent sees as repair authority. Natural-language correction text should be rendered from it.

## 8. Stage-Specific Refiners

Each stage owns a small refiner module. The common interface is:

```rust
trait StageRefiner {
    fn stage(&self) -> StageKind;
    async fn diagnose(&self, ctx: RefinerContext) -> Result<RepairDirective>;
}
```

### 8.1 ScopingRefiner

Reads:

- organizations tree;
- unit review / human approval audit trail;
- scoping stage spec;
- claims in attempted deliverable.

Outputs:

- missing organization creation;
- missing human approval;
- wrong `subject` on scope claims;
- submit-only guidance for confirm-only paths.

### 8.2 TargetIntelRefiner

Reads:

- `source_query_log`;
- `target_assets`;
- `dns_records`;
- WHOIS / RDAP source rows;
- provider availability and blocked/error rows;
- `recon_map_assets` / `recon_lookup_whois` tool skills.

Outputs:

- provider-only repair actions;
- source/query closure guidance;
- exact evidence refs for found/empty/blocked provider attempts;
- no scan-tool fallback unless stage policy allows it.

### 8.3 ExternalAttackSurfaceRefiner

Reads:

- in-scope targets / attack-surface seeds;
- active evidence ids;
- targets.ports, fingerprints, real_ip relationships;
- background jobs;
- freshness windows;
- EAS methodology and tool skills for `httpx`, `naabu`, `nmap`, `whatweb`.

Outputs:

- exact `(asset, technique)` actions;
- `found` vs `checked_empty` vs `blocked` vs `not_applicable` guidance;
- service-fingerprint denominator guidance;
- no broad rediscovery once coverage-gap repair is active.

Example from the latest failure:

```json
{
  "repair_kind": "coverage_gap",
  "root_cause": "Three non-resolving domain liveness cells were submitted as checked_empty; EAS policy should close these as not_applicable with a note and evidence.",
  "actions": [
    {
      "asset": "pa18.com",
      "technique": "GOLISH-EAS-LIVENESS",
      "expected_status": "not_applicable",
      "evidence_refs": [5232],
      "note": "Domain does not resolve; no liveness unit is testable."
    }
  ],
  "forbidden_tools": ["list_attack_surface_seeds", "list_in_scope_targets", "manage_targets", "bulk stdin", "CIDR sweeps"]
}
```

### 8.4 VulnTriageRefiner

Reads:

- findings;
- tested units / denominator;
- exploitability evidence;
- target data and stage policy.

Outputs:

- missing evidence refs;
- missing denominator/sample rationale;
- "run only this verification" actions;
- no unsupported exploit suggestions.

### 8.5 ReportingRefiner

Reads:

- per-stage completions;
- accepted deliverables;
- evidence ledger;
- report sections.

Outputs:

- missing sections;
- uncited claims;
- missing remediation or executive summary items.

## 9. ToolSkill Loading

ToolSkill should be deterministic and local. First slice can compose existing sources:

1. stage methodology (`resources/harness/stages/<stage>/methodology.md`);
2. `StageSpec.allowed_tool_types` and gate rules;
3. tool registry metadata and schemas;
4. curated tool usage cards, added later under a stable path such as `resources/harness/tool_skills/<tool>.md` or JSON.

The refiner uses ToolSkill to answer:

- which tool maps to this technique;
- what command shape is narrow vs broad;
- what output semantics count as found/empty/error;
- what terminal coverage status to submit for common negative cases;
- what tool patterns are forbidden during repair.

## 10. Persistence And Resume

`operation_state.state_blob.agent_run` already stores `submit_repair_mode` and runtime corrections. StageRefiner should persist the new directive there:

```json
{
  "agent_run": {
    "pending_gate_correction": "...rendered from directive...",
    "repair_directive": { "...": "structured directive" },
    "submit_repair_mode": { "...": "compat view for existing sub-agent executor" }
  }
}
```

Compatibility rule:

- `SubmitRepairMode` remains the current executor/tool-guard adapter in the first slice.
- `RepairDirective` becomes the source of truth.
- `SubmitRepairMode` is derived from `RepairDirective` until the executor consumes directives directly.

## 11. LLM Escalation

RefinerLLM is optional and bounded.

Input must be compact:

```json
{
  "stage": "external_attack_surface",
  "org_id": "...",
  "gate_reasons": ["..."],
  "current_gaps": ["..."],
  "last_submits": ["status counts only, not full payload"],
  "last_tools": ["tool, target, evidence id, outcome"],
  "db_snapshot": ["liveness facts, ports, fingerprints"],
  "stage_policy": ["terminal statuses and forbidden tools"],
  "question": "why is repair not converging?"
}
```

Output must be parsed into `RepairDirective` fields or rejected. Free-form prose is not authoritative.

## 12. Observability

Add a harness trace kind:

```rust
StageRefinerDecision {
    stage,
    agent_path,
    class,
    repair_kind,
    gap_count,
    action_count,
    llm_escalated,
    root_cause_preview,
}
```

Run tree should show:

```text
REFINER external_attack_surface/prober coverage_gap
  root_cause: checked_empty not accepted for non-resolving liveness
  actions: 3
  forbidden: list_attack_surface_seeds, bulk stdin
```

## 13. Migration Path

### P0: Mentor de-intervention

- Stop active mentor injection/blocking in repair mode and stage-run specialists.
- Keep traces only if useful.

### P1: StageRefinerCore

- Introduce common `RepairDirective`.
- Convert `coverage_gap_actions` to directive actions.
- Persist directive in `agent_run`.
- Keep existing `SubmitRepairMode` as executor adapter.

### P2: EASRefiner

- DB-backed diagnosis for liveness/port/service gaps.
- ToolSkill-guided status guidance for non-resolving domains and open-port fingerprinting.
- Use directive to reduce repeated repair submits.

### P3: TargetIntelRefiner

- Provider/source-query diagnosis.
- No scan-tool fallback for provider-only stages.

### P4: Outer Refiner Unification

- Route task-level unified refiner through StageRefiner.
- Preserve `SubmitOnly`, `Fabricated`, and `TextOnly` classification semantics.

### P5: Optional LLM Escalation

- Add bounded model call for stalled/ambiguous repair only.
- Reject unstructured output.

## 14. Open Decisions

1. ToolSkill storage format: Markdown cards, JSON, or both.
2. Whether repeated-tool telemetry survives after Mentor is removed.
3. Whether EAS negative liveness should normalize to `not_applicable` for DNS non-resolution or whether active Empty facts should later allow `checked_empty`.
4. Whether RefinerLLM should be a separate model setting or reuse the current chat model.

## 15. Recommended Decision

Adopt StageRefiner as the only active repair advisor. Remove Mentor active intervention from repair and stage-run paths immediately, then replace the remaining mentor path once DB-backed EAS/target_intel refiners exist.
