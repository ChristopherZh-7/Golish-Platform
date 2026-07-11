# Enumeration ownership and completion hardening

> Extends `docs/design/2026-07-10-enumeration-origin-terminal-closeout.md` after
> the first authorized Test1 run exposed additional cross-organization and
> partial-completion seams.
>
> **Current contract (2026-07-10):** this document supersedes the early draft
> assumption that the mere presence of a foreign subresource makes browser
> collection partial. A foreign request that is successfully blocked before
> dispatch is a visible scope exclusion, not unfinished authorized work.

## Context

The fresh Test1 session proved that exact-origin keys, evidence-backed terminal
rows, and non-terminal `partial` markers work in production. It also exposed
ways an active collector could outlive its authorized target binding, reuse
historical capture files as if they were current-run observations, or write a
terminal row after only part of the work completed. The current implementation
therefore treats ownership, provenance, completion, and evidence freshness as
one fail-closed contract rather than independent best-effort checks.

The user authorized autonomous completion of the remaining P0/P1/P2 work. The
schema change below is a row-preserving identity correction, but it is a
downtime-only migration rather than a rolling backward-compatible change:
writers must be stopped, the migration and new binary must be deployed
together, and an old binary must not be started against the migrated database.
It does not purge historical rows.

## P0 contracts

1. Every active Enumeration request is bound before network I/O to exactly one
   `scope='in'` target in the current workspace and runtime organization. A
   caller-supplied hidden organization is match-only and cannot replace the
   runtime organization. The requested URL must match an explicit target URL or
   a confirmed-open `targets.ports[].url` exact Web Origin.
2. Authorization is an immutable start-of-attempt snapshot of target id,
   organization, workspace, scope, exact origin, and the raw `name/value/ports`
   witness that granted that origin. A separate pre-write read is not enough:
   every target-bound business, timeline, evidence, attempt-marker, and terminal
   outcome writer locks the target and compares that witness in the same short
   database transaction that performs its write. `endpoint_add` additionally
   requires an unambiguous command exact origin
   and a unique authorized target before guarded `api_endpoints` /
   `crawl_observations` persistence; targetless/foreign crawler output never
   creates a scoped target. A deleted, moved, out-of-scope, de-authorized, or
   ambiguous target revokes the write instead of redirecting it to current
   mutable state. Network/AI work never runs while the DB lock is held.
3. Browser collection is strictly read-only: only `GET` and `HEAD` requests are
   allowed, click actions are disabled, and state-changing/dangerous route names
   are denied before navigation. Rust recipe sanitization and the Node helper
   both parse and double-decode path+query before accepting
   `recipe.manifest_paths`, `recipe.script_urls`, or `recipe.routes`; malformed,
   foreign-origin, and dangerous values are dropped before fetch/navigation.
   If a syntactically valid `%HH` escape remains after those two passes, the
   route is conservatively rejected as deeper/mixed encoding.
   Rust crawler, recipe, and route-probe guards share
   `ENUMERATION_DANGEROUS_ROUTE_TOKENS`, including builtin-wordlist state actions
   such as shutdown, restart, refund, and activate, so their policies cannot
   drift independently. The Node decoder replaces each valid `%HH` occurrence
   while preserving malformed neighbors such as `%ZZ`; one malformed escape
   therefore cannot abort both decode passes or hide an adjacent encoded token.
   Same-origin write-shaped XHR/fetch may be reported as blocked observations,
   but is never executed. WebSockets are never connected, including same-origin
   sockets, because the helper has no read-only message contract; their URLs
   remain visible scope exclusions.
4. Browser navigation, recursive fetches, and subresources are exact-origin.
   Foreign scripts, fetch/XHR, worker, image, font, and other
   subresources are blocked before dispatch and counted as visible
   `scope_exclusions`; successful exclusion alone does not make the authorized
   crawl partial. When an authorized main-frame request itself returns a
   terminal 30x to a foreign origin, the guard fetches only the authorized hop,
   never dispatches the foreign next hop, fulfills a local empty terminal
   document, and records `terminal_cross_origin_redirects`; that is a safe
   terminal observation and may close JS/JSAPI/PARAM as empty. A page-initiated
   foreign navigation, recursive-fetch escape, DNS/TLS/navigation failure, or
   failure to enforce the boundary remains a scope violation/error and forces a
   non-terminal result. Playwright navigation uses
   `route.fetch(maxRedirects=0)+route.fulfill`, so each later hop returns through
   the route guard instead of being followed implicitly. All manifest/recursive
   JS direct fetches, including auto-inferred candidates, share
   `fetchExactOrigin`; it validates both exact origin and the fail-closed
   encoded dangerous-route policy before the initial request and before every
   manually followed redirect hop.
5. `enum_crawl_same_origin_urls` accepts only confirmed-open exact origins. Every
   authorized input is canonicalized to the explicit-port root
   `scheme://host:port/` before list-file creation; caller path/query/fragment is
   discarded, while the bound `target_id` remains attached to the returned
   browser seed. Katana runs with redirects disabled and fixed rate,
   concurrency, parallelism, per-origin page, duration, and response-size
   bounds. Its exact-origin `-cs` union is a request boundary, and a fixed `-cos`
   generated from the shared Rust dangerous-route token set
   rejects dangerous path/query tokens in literal, percent-encoded, and
   double-percent-encoded form and broadly rejects remaining `%25` markers, so
   triple/deeper or mixed encoding cannot reach a request. Katana
   output is seed/provenance data rather than terminal coverage.
6. An active agent session cannot be replaced by a caller-provided `run_id`.
   Before expensive, networked, AI-assisted, or otherwise cancellable work, a
   producer atomically writes current-run `partial` attempt markers for every
   Enumeration axis it owns: browser = JS/JSAPI/PARAM, JS extraction =
   JSAPI/PARAM, and route probe = DIR. Failure to write the sibling markers
   prevents the attempt from starting. At completion, browser prepares all three
   sibling evidence/outcome writes and JS extraction prepares both sibling
   writes before either producer mutates `technique_outcomes`; each complete
   sibling group is then published with one target-guarded `upsert_batch`
   transaction. If any
   sibling cannot be prepared, that transaction publishes the whole group as
   `partial`; if publication itself fails, the attempt-start partial group
   remains authoritative. Mixed terminal/partial sibling groups are forbidden.
7. Any mixed transport, read, verification, queue, seed, manifest, or
   persistence failure prevents `found/empty`. Batch limits and malformed rows
   are never silently sliced away: browser/JS extraction return explicit
   omissions and write non-terminal markers for authorized overflow targets;
   route probe validates the entire batch and rejects an oversized or malformed
   batch before any request or marker write.
8. Terminal evidence is bound to the same session, organization, target,
   exact-origin asset, technique, outcome, and stage freshness cutoff. Evidence
   append first locks and validates the raw target witness, then serializes each
   operation hash chain with a transaction-scoped lock and commits the evidence
   row, hash link, and classification atomically. Attempt and terminal outcome
   batches acquire the same witness lock and verify their organization matches
   the target owner. Gate reads additionally require the evidence row's project
   path to match the target's current project binding, so moving a target cannot
   carry old-workspace evidence into a new denominator. A missing target
   binding, stale row, foreign-org/project row, or failed append cannot close a
   cell.
9. Browser manifests are rebuilt from scripts observed or revalidated in the
   current run; historical capture files are cache material only. The browser
   obtains `operation_id` from trusted `AgentToolContext`, passes it to the
   helper, and records it as `producer_operation_id` together with run/session,
   `captured_at`, and `producer_stage_started_at` from the current operation
   state. JS extraction uses its own trusted context id to load
   `operation_state`; it accepts the manifest only when the operation exists,
   is not superseded, has `current_stage='enumeration'`, matches the manifest id,
   and `captured_at >= operation_state.stage_started_at`, in addition to exact
   run/session matching. It then reads only that manifest's workspace-confined
   file allowlist, so an orphaned file from an older run or stage cannot become
   current terminal evidence. A page-budget checkpoint may carry cumulative
   scripts/API observations forward only for the exact same producer
   operation/stage-attempt/run/session: its parseable `producer_stage_started_at`
   must exactly equal current `operation_state.stage_started_at`, its
   `captured_at` must be at or after that timestamp, the sole prior incomplete
   reason must be `page_queue_remaining`, and every pending URL must be
   persisted. Other partial reasons, cross-operation/run/session manifests, and
   old checkpoints from an earlier stage attempt under the same operation never
   resume.
10. `technique_outcomes` identity is
   `(organization_id, run_id, asset, technique)`. The old key omitted the
   organization and allowed sibling organizations in one stage-run to overwrite
   each other. A forward migration replaces only the unique constraint; rows and
   columns are preserved.
11. Target-owned directory identity is `(target_id, url, tool)`. The legacy
    partial unique index omitted `target_id`, so one target could conflict with
    and update a sibling target's row for the same URL/tool. The forward
    migration rebuilds that row-preserving index, writers use the matching
    conflict key, and active route persistence also holds the target witness
    lock in the write transaction.
12. DIR runtime/request-budget partials persist a compact queue checkpoint in
    the trusted `operation_state.state_blob`, atomically partitioned by target
    and exact origin. Resume requires exact run, session, trusted operation,
    Enumeration `stage_started_at`, target/org/project authorization witness,
    origin, and deterministic plan hash. The plan hash includes only stable
    caller configuration, explicit observed inputs, and full wordlist content;
    it excludes DB-derived seeds written by the prior slice and the per-call
    runtime/request budgets. Cumulative request/match counts are evidence only,
    while each invocation gets a fresh local budget. Pending candidates resume;
    complete/error, owner drift, stage/run drift, and invalid checkpoints clear
    or reject the slot fail-closed. Candidate-plan/wordlist truncation remains
    visibly partial and can never become a false empty completion.
    The immutable target snapshot is also revalidated after each rate-limit
    wait and immediately before every candidate and uncached baseline request. Drift stops
    the loop before that launch, clears the checkpoint, and leaves DIR
    non-terminal; request counters never claim a rejected batch was sent.

## P1 contracts

- Current-wave worklist, submit preview, org gate, and pass-token denominator use
  the same asset set; next-wave and dead assets cannot be reintroduced by a
  coverage snapshot.
- Route probing builds a deterministic finite plan: curated checks may follow
  observed prefixes, while the full wordlist runs once at root and recursively
  only beneath verified directories. Runtime/request caps are invocation-local
  slices with durable same-attempt resume; completion prompts omit both fixed
  caps and let the default queue drain. Explicit caps remain diagnostic and
  honestly partial until the saved queue is exhausted.
- A custom route wordlist must resolve to a canonical regular file at either
  `<workspace>/1.txt` or below `<workspace>/.golish/wordlists/`. Other workspace
  source/config/secret files, parent traversal, absolute/external paths, symlinks
  and symlink escapes, and dangerous state-changing candidates are rejected before queueing. A
  wordlist-entry cap is reported as candidate-generation truncation and therefore
  remains non-terminal.
- Tool detection prefers an explicit command match and only falls back to stdout,
  so Katana URL output cannot be mislabeled as gau.
- Crawler output is seed/provenance only. Until background `JobCompletion` can
  retain the immutable launch target witness, this wrapper is foreground-only,
  kills on timeout, disables generic structured/evidence landing, and returns a
  bounded browser seed. It must never re-resolve a mutable owner at completion.
  Each bound root carries the full raw `TargetAuthorizationSnapshot`; after
  tool/config/command/list-file preparation, `PentestRunTool::execute_guarded`
  revalidates every `TargetWriteGuard` at the final pre-spawn seam. Owner/scope/
  project/name/value/ports drift therefore launches zero Katana requests.
  That seed always contains every authorized input root, even when Katana found
  no extra route for one root; source truncation and roots without a Katana seed
  are explicit diagnostics, and the browser still visits each root. Truncated
  stdout is `output_truncated_partial` seed material with
  `outcome_persisted=false`, never terminal Enumeration coverage.
- The four EAS wrappers are foreground-only. Their legacy `background` field is
  ignored, and a call returns only after guarded landing plus evidence/outcome
  publication. Prober never fans these wrappers into jobs or waits/kills a
  background copy; partial/error cells are refreshed from the worklist and
  retried as bounded foreground batches.

## Validation

- RED-to-GREEN unit and integration tests for every P0 seam.
- Two-organization outcome identity test after migrations.
- Local dual-server tests prove foreign subresource request count remains zero,
  exclusions are observable, and clean exclusions do not alone change a
  complete result to `partial`.
- Read-only browser tests prove non-GET/HEAD and dangerous routes never reach the
  server, recipe manifest/script/route encodings cannot bypass the filter,
  guarded redirects never issue a foreign/dangerous next-hop request, and no
  WebSocket reaches upgrade. Crawler tests pin root-only list input, retained
  target binding, dangerous-link zero hits, and redirect/rate/concurrency/page/
  duration/size bounds.
- A finite multi-page fixture proves same-operation/run/session page-budget calls
  eventually close with cumulative scripts/API observations, while a different
  operation, run, or session cannot reuse even a valid partial checkpoint; real
  link truncation stays non-terminal.
- Cancellation, sibling-prepare/publish failure, oversized-batch, target-move,
  stale-evidence, concurrent evidence append, and stale operation/run/session/
  capture tests prove the fail-closed contracts above.
- Full selected crate tests, Clippy, frontend checks, and `just precommit`.
- A new authorized Test1 session verifies exact-origin rows, positive evidence
  IDs for found/empty, non-terminal partial/error, worklist/gate parity, and a
  deterministic final gate result.
