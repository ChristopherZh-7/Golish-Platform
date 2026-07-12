# Target Surface Fingerprint / Network-Failure Closure

Status: Approved for implementation by the user on 2026-07-12; focused live acceptance pending.

## Problem

The current flow has four independently reproducible gaps:

1. `directory_entry_list` joins `directory_entries` and `targets` while selecting
   unqualified `id`/`content_type`/`created_at`, so PostgreSQL rejects the query as
   ambiguous.
2. Target Surface loads every optional source in one `Promise.all`; one directory
   failure erases fingerprints, origins and hierarchy that loaded successfully.
3. WhatWeb fingerprints are durable, but their evidence does not carry the exact
   canonical Web Origin. The frontend also drops legacy object-shaped evidence,
   attaches a multi-origin fingerprint to only one origin, and renders confidence
   values in the database's `0..1` scale as `0..1%`.
4. A target-attributed transport failure can either loop forever or become terminal
   after one observation. Neither behavior implements the requested three-attempt,
   auditable exclusion contract.

## Decisions

### 1. Query and read-model isolation

- The joined directory query must project only `de.<column>` fields.
- Each Target Surface source has an independent loading result. A failed optional
  source remains an explicit section error; successful sibling data stays visible.
- A page-level error may summarize partial failures, but must never replace already
  loaded data with `EMPTY_SURFACE_DATA`.

### 2. Exact-origin fingerprint evidence

- New WhatWeb fingerprint evidence is an array of observations. Every observation
  includes the producer and canonical exact `origin`/`url` used for that run.
- The existing `(target_id, category, name)` identity remains unchanged. If the same
  technology is observed on multiple origins, the upsert preserves all distinct
  origin observations instead of inventing duplicate fingerprint identities.
- The frontend accepts both legacy object and current array evidence, and attaches a
  fingerprint to every explicitly matching origin.
- Legacy fingerprints without origin evidence remain visible in a target-level
  `Unassigned fingerprints` view. They are never guessed onto a sibling origin.
- Confidence accepts both legacy percentages and canonical `0..1` values, displaying
  both as a bounded percentage.

### 3. Three-attempt transport breaker

The retry identity is scoped by the current operation epoch:

`operation_id + stage_started_at + organization_id + target_id + exact_origin + technique + failure_class`.

- Count only backend-classified, target-attributed connect/timeout/refused/reset/EOF
  and TLS-handshake failures. HTTP status responses prove reachability and reset the
  matching counter. Empty WhatWeb technology output is `empty`, not a network error.
- Tool/runtime/configuration/database errors, truncation and unbound stderr never
  increment the counter.
- Attempts 1 and 2 persist guarded `error` outcomes with `attempt=1|2` and remain
  retryable. Attempt 3 stops further WhatWeb work for that exact origin and records
  the stable failure class and all evidence references.
- Counters live in a namespaced JSONB slot in the existing `operation_state.state_blob`.
  Updates are atomic, guarded by `current_stage/stage_started_at`, and do not overwrite
  graph-flow or other harness state. A new operation or new EAS epoch naturally resets
  the breaker; a successful WhatWeb observation before the threshold clears its slot.
  Once attempt 3 makes the producer terminal, later same-epoch calls are short-circuited
  before network launch; only a new trusted epoch can reopen that producer.
- No target, port, Web Origin or evidence row is deleted. No open port is rewritten as
  closed, and sibling Host/SNI origins remain eligible.

### 4. Downstream exclusion requires independent confirmation

Three WhatWeb failures prove that the fingerprint producer cannot reach the origin;
they do not alone prove that every Enumeration producer is unable to reach it.

- On the third stable WhatWeb failure, EAS runs the existing fixed, tool-independent
  direct/proxy HEAD + bounded GET transport policy before stage handoff.
- If any policy receives an HTTP response, the origin remains eligible for
  Enumeration and only the WhatWeb producer is terminal for this operation; the
  downstream exclusion handoff stays clear, but same-epoch WhatWeb remains stopped.
- If every independent policy fails with target-attributed transport/TLS reasons,
  EAS persists a trusted exact-origin `web_origin_transport_blocked` handoff with the
  confirmation evidence.
- Enumeration input construction excludes only origins carrying that fresh, guarded
  handoff for the same operation/org/target/origin. It does not exclude an entire IP,
  domain or port and does not rerun the independent preflight in the next stage.
- A generic/model-authored `blocked` outcome, or a WhatWeb-only failure, can never
  suppress downstream work.

### 5. Target-visible failure evidence

- Attempts 1/2 and the attempt-3 handoff already live in target-bound audit evidence;
  the Target Evidence view must translate that structured payload instead of showing
  a generic completed audit row or no fingerprint.
- For `eas.fingerprint_web_stack`, display the canonical origin, failure class,
  attempt number and producer outcome. Attempt 3 says that WhatWeb is stopped for
  this EAS epoch; only `independently_confirmed=true` says the exact origin was also
  excluded from Enumeration.
- Use the existing timeline/detail payload. Do not add a schema, IPC command or
  generated type merely to render this status.

## Failure semantics

| Observation | EAS result | Enumeration routing |
|---|---|---|
| HTTP response, including 4xx/5xx | reachable | include |
| WhatWeb completes with no technology | `empty` | include |
| stable transport/TLS failure, attempt 1/2 | guarded `error` | not decided yet |
| stable failure attempt 3; independent probe reachable | WhatWeb producer `blocked` | include |
| stable failure attempt 3; independent probe also unreachable | transport handoff `blocked` | exclude exact origin |
| unknown/tool/DB/truncated/unattributed error | nonterminal error | include/fail closed |

## Invariants

- No migration, generated IPC type or external-tool exposure change.
- Every terminal/exclusion decision is target-bound, exact-origin-bound and backed by
  durable evidence.
- `checked empty`, `unreachable` and `not checked` remain distinct.
- A partial Target Surface error cannot hide unrelated durable data.
- The change is tested without launching a real external scan in this development
  session.
