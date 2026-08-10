# Fresh-install V2 defaults and Agent tree recovery

## Problem

The recovered worktree contained the durable Stage Team backend but had three recovery gaps:

1. a pristine desktop database froze new operations with legacy rollout contracts, so `stage_run`
   returned `STAGE_TEAM_V2_RERUN_REQUIRED` before creating a Unit, Controller, or Worker; and
2. the final Stage detail workspace remained only in the old dirty worktree. The recovered UI fell
   back to the earlier organization/Controller cards instead of the final Controller → SubAgent →
   tool-call tree and selected-Agent transcript; and
3. the current Target Intel Goal Loop persisted candidate observations without creating formal
   Targets, while the pre-EAS human scope review still read only the historical Target table.
   Target Intel therefore passed correctly but the operation stopped with zero active-recon
   authority.

These gaps made the frontend look missing even though the final design and backend read model still
existed elsewhere in the recovery evidence.

## Decision

### Pristine database defaults

An additive migration may select the accepted full-chain defaults only when `operation_state` is
empty and all five rollout singletons exactly match the known pristine legacy values. It locks
`operation_state`, locks the singleton rows, applies exact compare-and-swap updates, restores every
guard trigger, and writes one append-only content-addressed bootstrap receipt.

Existing deployments are never promoted by this migration. Any non-empty `operation_state` is a
no-op; any empty database with an unknown singleton combination fails closed. Existing operations
retain their immutable frozen contracts.

The selected defaults are runtime/attack `v2_only`, Enumeration `agent_team_v2`, Tool Truth
`receipt_v1`, and Investigation `hypothesis_registry_v1 + new_only`, which maps to
`unified_investigation_v1`.

### One Stage Agent workspace

`stage_run` and its SubAgent cards share one Tool Detail route. `StageRunOrgRows` accepts only one
exact operation/execution pointer and mounts the DB-backed Stage Team read model. The workspace
renders a left Agent tree (organization, Primary Controller, dynamic and retry Workers) and a right
selected-Agent surface (task, plan, thinking, response, tool calls, and durable evidence details).

The DB read model remains scheduler truth; Chat events only supply the visible transcript for the
exact Worker request identity. Missing or mixed pointers remain rerun-required. Candidate,
Investigation, Reporting, and Cleanup keep their typed detail views and are not replaced by this
workspace.

The recovered Target type and Target detail field renderer remain byte-identical to the last dirty
main tree. The one missing follow-up projection is restored separately: a resolution-only IP group
(a domain has `real_ip`, but no standalone IP Target exists) is selected from `buildHostTree`, not
the organization-only navigation tree, so its synthetic IP workbench and related domains remain
reachable.

### Candidate observations into explicit active-recon authority

The TargetIntel→EAS review now combines the currently trusted Target rows with unreviewed semantic
observations from the exact operation, organization, and current Target Intel execution window.
Only canonical domain, IP, CIDR, and web-origin observations with an admissible disposition can be
presented. Misclassified values, promoted rows, rejected/shared/third-party observations, foreign
operations, and older windows remain excluded.

The review does not promote observations by itself. A human-selected candidate is materialized as
a customer-provided in-scope Target; an unselected candidate is materialized out of scope. The
transaction rechecks the candidate witness, rejects duplicates and drift, and records a cumulative
review marker. Passive Intel evidence therefore remains candidate-only until a separate explicit
active-scan authorization exists.

Both the normal TargetIntel→EAS transition and a graph cursor restored exactly at EAS entry use the
same review-capable guard. A direct EAS slice has no current Target Intel candidate window and still
requires prior trusted authority; a restored crossing can emit the pending review before the
read-only trusted-target preflight returns a hold.

## Safety and compatibility

- No existing operation or non-pristine deployment is rewritten.
- The migration is additive and records an immutable receipt.
- SubAgent-card navigation resolves an owning `stage_run` only from persisted request identities.
- The obsolete standalone SubAgent detail and private Stage asset panel are removed; historical
SubAgent identities resolve through the unified Tool Detail route.

The typed Company Controller workspace replaces the generic Tool envelope rather than being
inserted into it. The standard `TOOL / INTENT / INPUT / OUTPUT` blocks and the duplicate running
footer are suppressed for that selected `stage_run`. The remaining Agent workspace owns the full
available height; its transcript is a bounded internal scroll viewport, so streaming output never
grows the whole detail page.

### Message-scoped ChatPanel plans

Each entered harness stage freezes the first assistant message created for that stage as the owner
of its inline Plan card. Projected future-stage seeds never manufacture transcript cards, and later
plan versions update the original card without moving it. Completed cards collapse in place and
remain reviewable. A separate compact status strip stays below the conversation tabs and exposes
the full workflow only through an explicit control. Message anchors persist with the roadmap and
merge safely on restore without replacing a newer live plan.
- Semantic observations never become active-scan authority without the exact scope-review choice.
- No external provider or target request is needed to validate this recovery.

## Verification

- isolated embedded-Postgres migration test for the exact pristine defaults and receipt;
- static migration guard test for pristine-only, append-only, audited behavior;
- focused Stage workspace, navigation, transcript, and Tool Detail Vitest suites;
- focused message-anchor, Plan-card, persistent status-strip, and roadmap restore suites;
- TypeScript no-emit and scoped Biome checks;
- read-only inspection of a fresh GUI operation proving unified frozen contracts and actual
  Controller/Worker rows.
