# Investigation asset Primary dynamic team

> **Status:** Approved for implementation on 2026-08-14.
>
> **Supersedes:** the fixed browser/researcher/pentester/adviser roster, fixed-role ordering and
> exact-role barrier in `2026-08-13-investigation-company-asset-queues.md`. It preserves that
> document's strict company and asset queues, exact asset ownership, dynamic Tool Manager,
> hypothesis-resolution gate and durable recovery requirements. It also restores the original
> dynamic-committee decision in `2026-08-12-investigation-primary-led-verification-execution.md`.

## 1. Product contract

Investigation has ordered business queues and a dynamic cognitive team:

```text
company queue (strict)
└─ current company
   └─ asset queue (strict)
      └─ current asset: one durable Asset Primary
         ├─ hypothesis-formation rounds: Primary dynamically delegates 0..N subtasks
         ├─ hypothesis-verification rounds: the same Primary dynamically delegates 0..N subtasks
         ├─ dynamic Tool Manager/browser invocations during verification
         └─ new-hypothesis discoveries return to the same asset backlog
```

Only company and asset membership are queues. Roles are not a queue, a fixed committee, a census,
or an authorization source. The Primary may choose any allowed specialist, repeat one specialist,
run independent specialists concurrently, revise the remaining plan, or finish a bounded round
without delegating. Browser, researcher, pentester and adviser remain useful role choices, alongside
coder, installer, enricher and memorist; none is mandatory.

## 2. One durable Primary per asset

An Asset Primary owns one durable message chain for the complete asset lifetime. It survives
analysis, verification, discovery admission, hypothesis revision and recovery. A new hypothesis
does not create a replacement Primary. A new asset does.

The Primary operates in two explicit durable modes:

- `hypothesis_formation`: read frozen current-asset evidence, predecessor products, CyberStrike and
  Knowledge Base; delegate read-only analysis; freeze canonical hypothesis revisions.
- `hypothesis_verification`: load one current unresolved canonical revision; delegate fresh
  reasoning or execution subtasks; use real managed tools when useful; submit a terminal resolution
  or continue the strategy.

The mode boundary prevents a proposal from silently changing during verification. The Primary's
history remains continuous, while every verification round names the exact frozen revision and
fresh child identities.

## 3. Dynamic delegation

The Primary authors a bounded plan rather than receiving host-authored role WorkItems. A plan may
contain zero to eight subtasks. Every subtask contains:

- a stable key and ordinal within the current round;
- one allowed role hint;
- a bounded objective and rationale;
- exact current-asset subject references;
- a mode (`hypothesis_formation` or `hypothesis_verification`);
- for verification, the exact current hypothesis revision.

The host validates the plan but does not choose its roles. It creates WorkItems only for accepted
subtasks. A Refiner may add, remove, retry, replace, reorder or run independent remaining subtasks,
subject to the round budget and exact asset/revision authority. Completed output is immutable and is
returned to the same Primary. Infrastructure failure is an observed result; it does not trigger a
role-specific automatic retry. The Primary chooses retry, replacement, another role or termination.

Zero delegated subtasks is legal. In formation mode the Primary must either submit canonical
proposals or an exact zero-hypothesis result. In verification mode it may resolve without a tool
only when the frozen authority legitimately supports `invalid` (for example, an exact duplicate or
wrong premise); absence of execution can never manufacture support or refutation.

## 4. Hypothesis formation

All model-visible context is asset-scoped. Cross-asset material is excluded or retained as a typed
non-authoritative signal for the later lane. Formation actors can read current-asset evidence,
CyberStrike, Knowledge Base, memory and graph projections, but cannot perform target I/O.

Each canonical proposal freezes its asset identity, structured claim, prerequisites, impact, proof
conditions, refutation conditions, initial strategy and cited authority. Once admitted, verification
targets the exact revision; changing its claim creates a successor or a new hypothesis.

## 5. Hypothesis verification

The same Asset Primary selects the next unresolved current canonical hypothesis in stable server
order. It dynamically delegates fresh reasoning and execution subtasks. Any accepted execution
subtask may call the real Tool Manager inventory or managed browser zero or more times. Roles do not
grant tools; the exact session, asset, target, hypothesis, worker fence, scope, authorization, budget,
credential boundary and current Tool Manager inventory do.

Tool Manager is the sole tool catalog. Investigation has no static browser/HTTP/Nuclei/sqlmap/script
capability enum and no exact-one-tool assignment. A child may list tools, read managed skills, use
the managed browser, run an installed and ready tool, inspect results and change tactics. External
I/O remains outside database transactions and every invocation remains replay-safe and auditable.

The Primary submits the hypothesis resolution. Independent child output and durable observations
remain available to challenge it, but no named Adviser or fixed role is required. The deterministic
host validates ownership, current revision and cited observation authority.

## 6. Discovery and completion

Verification may emit new proposals. The host derives current-asset identity and semantic keys,
dismisses exact duplicates, and admits real new proposals through the canonical compiler. Every
unconsumed discovery keeps the current asset open.

An asset may advance only when:

- formation has a typed zero-hypothesis authority; or
- every current canonical hypothesis is `verified`, `refuted` or `invalid`;
- no current-asset hypothesis discovery remains unconsumed.

`open`, `untested`, `inconclusive` and `blocked` are unresolved. Tool names, role names, role counts,
subtask counts, tool-call counts, Campaigns, Waves, receipts, Oracles and FactDelta cardinalities are
not business completion gates. They remain evidence and audit data.

## 7. Recovery and cutover

New and resumed runnable Investigation uses this single contract. Fixed-roster schedules and
fixed-role verification rounds remain historical audit rows only. Runtime never resumes them as
authority and has no compatibility fallback.

On recovery the host reloads the current company, asset, Primary chain, mode, round plan, completed
outputs, current hypothesis and invocation states. It does not restart predecessor stages, refreeze
queues or infer a mandatory next role. Unknown external outcomes remain held for explicit recovery
and are never blindly replayed.

## 8. Acceptance

Acceptance requires the retained Investigation-only entity operation to demonstrate:

1. the current asset has one continuous Primary chain;
2. Primary-authored delegation does not equal a fixed four-role set or fixed order;
3. canonical hypotheses are formed from current-asset evidence and knowledge;
4. the same Primary enters verification and dynamically uses real Tool Manager/browser capabilities;
5. new verification discoveries are admitted or dismissed as duplicates on the same asset;
6. every current hypothesis reaches `verified`, `refuted` or `invalid` before the next asset;
7. crash/resume does not restart upstream work or duplicate external action;
8. all assets and companies advance in their frozen order to the resolution-only stage closure.
