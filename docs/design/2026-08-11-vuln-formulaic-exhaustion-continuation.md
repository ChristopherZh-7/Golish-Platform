# Vuln formulaic exhaustion continuation

## Problem

The server-owned Vuln worklist can finish every child WorkItem while one bounded
anonymous-access probe remains `partial` because its narrowed final attempt hit
request or batch timeouts. The current Nuclei-only exhaustion terminalizer does
not recognize that evidence. The Company Controller may then exhaust its own
lease fuel, leaving an open TeamPlan with no runnable child and no path to the
deterministic final submission. Repeating a user `continue` request returns to
the same durable state.

## Required behavior

1. Preserve the original anonymous-access evidence and its `partial` outcome.
2. Only after the exact server-authored narrowed recovery attempt is terminal,
   append a derived residual evidence row and advance the one WSTG-ATHN-04 cell
   to `blocked` (attempted but inconclusive), never `checked_empty`.
3. When that transition leaves the Vuln denominator fully terminal, re-arm only
   the original exhausted Company Controller so it can close the request epoch
   and perform final submission. Do not replay a child tool or broaden scope.
4. Response-loss replay must return the same terminal projection or the same
   Controller generation; partial, foreign, malformed, or non-timeout evidence
   remains fail-closed.

## Exact admission witness

Anonymous residual sealing requires all of the following in one transaction:

- current active `vuln_triage` execution and frozen V2-only operation;
- exact organization, Unit, open Company Controller TeamPlan and exhausted
  `leader:primary`/failed Worker lease-expiry output;
- all required child WorkItems terminal with exactly one output each;
- one current `technique_outcomes` cell for the exact origin and
  `WSTG-ATHN-04`, sourced by `vuln_probe_anonymous_access` and pointing to one
  in-scope evidence row;
- `anonymous_access_batch_v1`, matching target/origin/technique, reviewed and
  selected counts, at least one network attempt, no suspicious/found verdict,
  and only request/batch timeout residual classes;
- producer identity bound to the same operation/execution/Unit/org and to the
  exact terminal child Worker/output;
- accepted dynamic request objective `vuln_formulaic_shard.v1` with capability
  `anonymous_access`, tool `vuln_probe_anonymous_access`, exact target/origin,
  exact single technique, `shape=narrowed`, and recovery attempt equal to the
  server maximum.

## Controller recovery

After residual sealing, the ordinary leader claim may perform one narrow
in-place recovery when the open formulaic TeamPlan has no nonterminal
required child, no live/recovery Worker, and no current pending/partial/error
Vuln cell. A forward migration permits only this witnessed failed-to-running and
exhausted-to-running pair. The recovery increments the existing Worker attempt
epoch and writes a server-only checkpoint marker; it does not create a new
message chain, clear evidence, or invoke a producer. The next runtime pass sees
zero unfinished cells and enters the existing prepare-final path.

## Non-goals

- Treating arbitrary scanner/runtime errors as terminal.
- Converting timeouts into negative findings.
- Replaying HTTP requests or Nuclei automatically after exhaustion.
- Relaxing exact target, scope, evidence, lease, or Gate validation.
