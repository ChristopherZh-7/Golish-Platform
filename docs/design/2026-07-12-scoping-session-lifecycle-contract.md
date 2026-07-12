# Scoping session lifecycle and trusted-review contract

## Problem

The live `pentest-chat-1783786690236-1` run accepted two Scoping deliverables, but
the authoritative close gate retried the whole stage. TaskMode stored the durable
chat session as one UUID while `DbTracker` continued writing `tool_calls` under the
random UUID created during bridge setup. The gate queried the durable TaskMode
session and therefore could not verify the completed `unit_review` / `scope_review`
lifecycle.

Two contract mismatches compound that failure:

- trusted UI intake writes target rows with `source=customer_provided`, while the
  Scoping snapshot whitelist omits that source;
- the close gate requires a non-empty `scope_review` even for company-name,
  organization-only engagements whose trusted target snapshot is empty, despite
  the Scoping methodology declaring `unit_review` to be the only review there.
- the lifecycle read keeps only the latest `scope_review`, allowing a second
  confirmation to erase an earlier edit/rejection;
- project-wide value dedupe can silently discard a `customer_provided` import
  when a provider or sibling organization already stored the same value.

The next live run, `pentest-chat-1783791527002-1`, proved the durable UUID fix
worked (all 17 lifecycle rows used the TaskMode session), but exposed a separate
flow conflict: the human explicitly chose parent-only scope while the red-team
post-check still unconditionally demanded `propose_candidates + unit_review` and
retried the entire stage. Ordinary choice prompts also auto-selected their first
option after 60 seconds, which is not acceptable evidence for a scope boundary.

## Decisions

1. `DbTracker` owns a shared, synchronously readable session identity. All tracker
   clones observe a TaskMode rebind. After `sessions::upsert_by_chat_key` returns,
   TaskMode binds the tracker to that exact UUID before any stage tool can run.
   A `ToolCallGuard` snapshots the UUID at start so the matching finish row cannot
   drift if a later request rebinds the shared tracker.
2. A non-empty trusted target snapshot requires a persisted, parseable, exact
   `scope_review`; edits and malformed responses remain fail-closed.
3. An empty trusted target snapshot is an organization-only Scoping path. It does
   not manufacture or authorize targets and does not require an empty target-table
   review. Red-team `unit_review` remains independently required by policy.
4. `customer_provided` is a trusted pre-stage intake source, equivalent to the
   existing manual/imported/CLI seed sources, in both the Scoping snapshot and
   Target Intel's authorized-root query. Provider/discovery sources stay outside
   that tier.
5. A non-empty snapshot permits exactly one completed `scope_review` in the
   current operation window. All attempts are retained; a second dialog blocks
   instead of replacing the first human decision.
6. Org-bound customer imports dedupe only against the same exact target identity
   in the current org or a legacy unbound row. Legacy claim is CAS-guarded;
   sibling-org rows are never reassigned and receive a separate current-org row.
7. Red-team subsidiary scope has two persisted branches. A non-skipped choice
   explicitly excluding subsidiaries completes the root-only branch without
   discovery, candidate proposal, or an empty unit-review table. An inclusion
   branch requires a successful same-org `propose_candidates` lifecycle followed
   by a successful, parseable, non-skipped same-org `unit_review`.
8. New subsidiary choices carry structured context with both
   `decision=subsidiary_scope` and the trusted root UUID. The DB gate validates
   that binding; an exact-root-name fallback exists only for already-running
   legacy prompts. The frontend disables generic auto-confirm for both forms, so
   only an explicit click can establish the decision.

## Safety properties

- No target row is created or edited by Scoping.
- An empty snapshot authorizes zero executable assets; later stages may only work
  from persisted in-scope targets.
- Non-empty reviews are exact-matched on canonical value, target type, and scope.
- No schema, migration, or generated IPC type changes are required.

## Verification

- shared tracker-session identity regression test;
- Scoping pure tests for empty-snapshot no-op and non-empty exact matching;
- app DB-bridge SQL test for `customer_provided`;
- root-bound parent-only and ordered included-subsidiary lifecycle tests;
- frontend fake-timer regression proving subsidiary scope never auto-confirms;
- focused `golish-agent-kit`, `golish-agent-bridge`, and `golish-agent-app` tests;
- a no-GUI TaskMode/Scoping regression proving tool calls and gate lookup use the
  same session UUID.
