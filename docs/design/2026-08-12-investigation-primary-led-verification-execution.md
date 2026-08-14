# Investigation Primary-led verification execution

> Superseded in part by `2026-08-13-investigation-company-asset-queues.md`: new-contract
> Investigation uses a durable company queue, a durable asset queue, one Asset Primary across the
> asset lifetime, a required browser/researcher/pentester/adviser reasoning census, and an
> asset-scoped dynamic hypothesis backlog. This document remains authoritative for the governed
> execution assignment, PreparedAction/JIT, evidence and Oracle boundary.

> **Status:** Implemented and focused-accepted on 2026-08-12.
>
> **Supersedes:** the cognition-only Verification worker and host-only execution decisions in
> `docs/design/2026-08-02-rag-first-unified-investigation-stage.md` §§4, 8, 10–13. The unified
> Investigation stage, Hypothesis Registry, Prepared Action/JIT, Tool Truth, evidence ledger,
> Oracle, FactDelta, Reporting authority, frozen-scope and rollout boundaries remain in force.

## Implementation status（2026-08-12）

The Primary-led control plane and the first complete execution path are implemented. The runtime
has a closed actor-contract vocabulary with fail-closed tool exposure; Analysis synthesis keeps the
same Primary history, receives every child output and may seal a typed zero-hypothesis generation.
Verification intents select a broad worker role but never confer authority. The host materializes
an immutable, exact PreparedAction/JIT/WorkItem-bound execution assignment; a durable execution
worker claims it with a double Worker/assignment fence and sees only its one canonical tool call.

The controlled DirectoryFingerprint adapter now performs one candidate and three same-origin
soft-404 HTTP observations, lands a versioned non-raw capability receipt, and lets the database
recompute the Oracle result. Material support/contradiction FactDelta no longer becomes an
accidental fixed point: it creates an append-only pending evolution authority. A newly rearmed
Analysis Primary either applies a successor generation atomically (`advanced`) or closes the source
wave as a true fixed point without creating an empty generation. Exact replay does not repeat HTTP,
assignments, receipts, Oracle assessments, generations or consolidation receipts. The complete
user-supplied CyberStrike source corpus is copied, content-addressed, data-only, lazily queried and
included in the Tauri bundle.

The controlled acceptance suite now proves the real HTTP/receipt/Oracle/assignment path, the
Campaign→FactDelta→adjudication→pending-authority bridge, and both live-database evolution terminal
paths. Scoped Rust/TypeScript gates are green, including the fail-closed no-execution/no-Oracle,
scope, credential, adapter, replay and immutable-assignment checks.
Additional browser/CLI/script/PoC adapters remain capability extensions rather than prerequisites
for correctness of the first governed end-to-end execution contract.

## 1. Decision

Investigation uses one orchestration shape twice:

```text
Investigation Coordinator / Refiner
├─ Hypothesis Analysis Task
│  └─ Analysis Primary
│     └─ dynamically delegated read-only specialists
└─ one VerificationTask per scheduled hypothesis
   └─ Verification Primary
      └─ dynamically delegated execution specialists
```

The role catalog is broad and reusable (`pentester`, `researcher`, `browser`, `coder`,
`installer`, `enricher`, `memorist`, `adviser`). It is not a fixed committee. A Primary chooses
zero or more roles from the actual gap, delegates bounded work, observes each immutable result in
the same durable chain, revises the remaining plan, and decides when its task is terminal.

Browser, CLI, HTTP, scanner, script and PoC are tools, not Agent roles. Skills are versioned
methodology knowledge, not execution authority.

## 2. Authority matrix

| Actor | Context | Delegation | Tools | May establish truth |
|---|---|---|---|---|
| Investigation Coordinator / Refiner | sealed run and generation summaries | schedules Analysis and Verification tasks | control-plane only | no |
| Analysis Primary | full bounded analysis snapshot and all child results | dynamic read-only specialists | plan, memory, knowledge, graph, exact-scope reads | no |
| Analysis specialist | one frozen subtask | optional bounded read-only nested help | memory, knowledge, graph, exact-scope reads | no |
| Verification Primary | one exact hypothesis revision, plan, objective/Campaign set and accumulating execution receipts | dynamic execution or reasoning specialists | plan, dispatch, evidence reads; no direct attack tool | no |
| Verification reasoning specialist | one frozen strategy/prerequisite gap | optional bounded reasoning help | Skills/memory/knowledge/exact-scope reads | no |
| Verification execution specialist | one exact authorized execution assignment | bounded nested help only when the child authority permits it | host-admitted browser/CLI/HTTP/scanner/script/PoC tools | only by producing fresh ledger evidence |
| Oracle / Registry reducer | sealed execution and evidence census | none | deterministic host code | yes, within its typed contract |

Stage kind alone never decides a worker's capabilities. The runtime derives a closed actor
contract from the exact durable WorkItem and its host-owned execution binding. Missing, unknown or
inconsistent actor contracts fail closed to read-only; a role name can never grant tools.

## 3. Task contracts

### 3.1 Analysis

The Analysis Primary may produce zero hypotheses when the sealed input/checklist/methodology
census is complete. Zero means “no bounded hypothesis was formed”, never “the target is safe”.

Every child body, not just the last two outputs, is available to the same Primary for refinement
and final synthesis through a bounded lossless projection. Token pressure may compact prose, but
it must retain every output identity, proposal/action/residual object and hash. Final synthesis is
a continuation of the Primary's durable history, not a reset-history pseudo-agent.

Analysis workers cannot perform target I/O or write evidence/Finding/canonical hypotheses.
CyberStrike-style Skills are queried lazily by metadata, prerequisites, technique/CWE/product
tags and attack-chain relationships. Hits remain `knowledge_signal` and must be revalidated.

### 3.2 Verification planning

A Verification Primary receives one exact `HypothesisVerificationTask`, including its current
hypothesis revision, verification plan, Campaign/objective denominator, scope, policy, credential
availability, prior evidence and residuals. It dynamically decomposes unresolved obligations.

Reasoning work may select or challenge methods, identify prerequisites, request a Coder-designed
PoC shape, or ask an Adviser for an alternative. A proposed action remains non-executable until
the host compiler binds it to exact current authority.

### 3.3 Verification execution

The host compiles each accepted intent into an immutable execution assignment bound to:

- operation, stage execution, Unit, organization and frozen scope;
- hypothesis revision, VerificationTask, Campaign and objective;
- target selectors and any credential reference (never secret material in the prompt);
- capability/tool allowlist and canonical arguments or a bounded script workspace;
- risk class, JIT decision where required, budget, conflict key, lease and attempt epoch;
- expected evidence and negative-oracle contract.

Only then may the Verification Primary dispatch an execution specialist. The specialist sees tools
allowed by that assignment, not all Investigation tools. Every external call passes the normal
scope guard, worker tool lifecycle, send-before-begin journal, cancellation/unknown-outcome rules
and evidence landing. Writing a PoC/script is allowed only in a task-scoped artifact directory;
executing it requires the same external-action authority as any equivalent CLI call.

Prepared Action is therefore a security envelope, not a host replacement for the Agent. The Agent
chooses the strategy and tool sequence within the sealed assignment; the host owns authorization,
scope and evidence integrity.

## 4. Continuous hypothesis loop

Fresh verification evidence can produce a typed `HypothesisSignal`:

```text
parent_revision_id
source_verification_task_id
evidence_refs
relation = support | contradiction | derive | split | independent | merge_candidate
structured_claim
reason
recommended_methodology_refs
```

The outer Refiner/Registry reducer decides whether it updates the same revision, creates a derived
or independent hypothesis, merges a duplicate, or records a residual. A material decision creates
a successor snapshot/generation and automatic VerificationTask admission. No material delta writes
a fixed-point receipt and does not repeat the same attack.

## 5. Terminal semantics

- `proposed`: canonical hypothesis exists and has not been execution-adjudicated.
- `strategy_ready`: an executable or residual verification strategy is sealed; no attack is
  implied.
- `verified`: fresh execution evidence satisfies a proof path and the deterministic Oracle accepts
  it.
- `refuted`: fresh negative evidence satisfies every designated falsifier required by the plan.
- `inconclusive`: execution occurred, but the proof/refutation contract is not satisfied.
- `blocked`: credential, authorization, capability, environment or recovery authority is missing.

Zero Prepared Actions, zero executions and zero new evidence can never yield `verified`, `refuted`
or a semantically successful verification. Such a task remains `proposed`, `inconclusive` or
`blocked` with an explicit residual. `pass_with_gaps` may close the bounded Investigation run, but
must not visually or canonically upgrade those hypotheses.

## 6. Skills integration

The repository includes the complete user-supplied CyberStrike Skill source tree as a separate,
unmodified third-party corpus under `resources/methodology/corpora/cyberstrike/`. The copy retains
the upstream `LICENSE`, `README.md`, package manifest and all 7,750 files under `.cyberstrike/skill`
(7,660 `SKILL.md` documents). `GOLISH_PROVENANCE.json` records that the Downloads copy has no Git
metadata, so its exact identity is the copied tree hash rather than an invented commit.

The corpus remains AGPL-3.0-only third-party material. Golish must preserve its license and source
provenance when distributing it. Import/index code must not rewrite the original source files.

- Analysis queries skill metadata, triggers, prerequisites, “what to check” and chains.
- Verification reasoning queries full methodology, payload families, tool requirements, negative
  oracles and evidence requirements.
- Execution may use a method only after mapping it to a Golish capability and exact authorization.
- Missing/unsupported adapters become explicit residuals, not empty checks.
- The bundled source is admitted only through an explicit corpus policy for this exact content
  root/license/provenance tuple. It is never discovered by the ordinary workspace Skill provider
  and never injected wholesale into a system prompt.

## 7. Recovery and compatibility

Existing frozen `unified_investigation_v1` operations retain their cognition-only contract and are
never reinterpreted. The new behavior is selected by a monotonic operation-frozen contract version.
If persistence changes are required, migrations are additive and readers remain dual-version until
old operations are terminal. Network/provider work never occurs inside a database transaction.

Primary chains remain durable across child calls. Response-loss replay accepts only the exact
WorkItem/Worker/chain/output/action/evidence census. Unknown external outcomes are held for operator
recovery and are never automatically replayed.

## 8. Delivery slices

1. Add the actor-contract vocabulary, fail-closed derivation and tests without changing existing
   operation behavior.
2. Separate Analysis and Verification task/worker contracts; remove limited-output synthesis and
   reset-history reduction.
3. Add Verification Primary execution assignments and guarded tool exposure, initially for safe
   fixture/loopback adapters.
4. Connect Prepared Action/JIT, real browser/CLI/HTTP/script adapters, evidence/Oracle/FactDelta and
   derived-hypothesis admission.
5. Index the bundled CyberStrike corpus and add lazy governed retrieval, then controlled fixture
   and fresh entity acceptance.

## 9. Acceptance

The feature is not complete until a controlled authorized run proves all of the following:

1. Analysis actors expose no target-I/O tool and can close with zero hypotheses plus a bounded
   residual/census.
2. A Verification Primary dynamically delegates at least two independently useful roles without a
   fixed roster and keeps one durable context through their results.
3. At least one execution specialist performs fresh browser/HTTP/CLI or script-backed work against
   a loopback/controlled target and books exact evidence.
4. Missing JIT, credential, adapter or scope blocks before send and is visible as a typed residual.
5. Fresh support/refutation evidence changes the revision only through Oracle/Registry authority;
   a new independent claim creates a successor generation/task.
6. Zero execution cannot be presented as verified or refuted.
7. Restart/replay does not duplicate an external action, child, evidence row, hypothesis or
   Campaign.
8. The bundled CyberStrike copy contains exactly 7,750 skill-tree files and 7,660 `SKILL.md`
   documents with tree hash `b2aa092cf37cce491a86c765f0231f78e2a78510ef4f101ab393e422019a3331`;
   license/provenance are present and runtime retrieval is bounded and content-addressed.

## 10. Non-goals

- No free-form autonomous attacks outside the frozen engagement scope.
- No role-name-based privilege and no fixed role committee.
- No model-authored authorization, credential, canonical target, Finding or verdict.
- No treating third-party Skill prose as system instruction, scope, authorization or proof.
