# Enumeration ownership and completion hardening plan

> This is the implementation and acceptance sequence for the current contract in
> `docs/design/2026-07-10-enumeration-ownership-and-completion-hardening.md`.
> The earlier assumption that every foreign subresource makes a crawl partial is
> superseded: a request successfully blocked before dispatch is a visible scope
> exclusion; an unenforced or escaped boundary remains non-terminal.

1. Add RED tests for target/workspace/org/exact-origin/run binding. Snapshot the
   authorized target before active work, make hidden org/run arguments
   non-overriding, then lock and compare the raw target witness in the same
   short transaction as every target-bound business/evidence write. Include the
   generic crawler output-store path: command origin must map uniquely through
   target URL/confirmed-open `ports[].url`; guarded endpoint/observation writes
   reject owner drift and cross-project/org conflicts without holding a DB
   transaction across network work.
2. Add dual-server browser tests for foreign navigation, subresources, fetch/XHR,
   websocket, and recursive fetches. Count cleanly blocked foreign subresources
   as scope exclusions without making an otherwise complete crawl partial; keep
   navigation/recursive escapes and enforcement failures non-terminal.
3. Enforce a read-only browser policy: allow only `GET`/`HEAD`, disable clicks,
   deny dangerous/state-changing routes, and retain blocked same-origin API
   observations without sending the write-shaped request.
4. Pin crawler safety in tests and implementation: accept only confirmed-open
   exact origins, disable Katana redirects, and fix rate, concurrency,
   parallelism, page, duration, and response-size bounds. Preserve crawler output
   only as bounded browser seed/provenance data.
5. Add atomic start-of-attempt `partial` markers before cancellable work for each
   producer's complete axis set. Add mixed transport/read/verification/queue/
   seed/persistence tests and forbid `found/empty` unless the whole declared unit
   completed.
6. Make every batch input auditable. Browser/JS extraction must report rejected,
   skipped, and over-cap rows and write non-terminal markers for authorized
   overflow; route probe must reject an invalid or oversized batch before any
   sibling starts requests or writes markers.
7. Make route candidate construction finite and safe. Resolve custom wordlists
   inside the canonical workspace, reject external/symlink escapes and dangerous
   routes, run the full wordlist once at root, recurse only under verified
   directories, and report any request/wordlist cap as `partial`.
8. Bind terminal evidence to session, organization, target, exact-origin asset,
   technique, outcome, and the current stage freshness cutoff. Serialize each
   operation's hash-chain append and commit evidence plus classification in one
   transaction.
9. Rebuild browser manifests from current-run observations/revalidations only.
   Treat historical files as cache candidates, and constrain JS extraction to
   the current manifest's workspace-confined file allowlist. Add a two-run stale
   capture regression.
10. Replace the `technique_outcomes` unique key with the organization-scoped key
    through a forward migration; align current-wave/dead-asset denominator
    construction across read model, preview, org gate, and pass token.
11. Fix command-first producer detection, retained crawler seeds, and
    agent-facing methodology/worklist copy so `partial`/`error` remain visibly
    unfinished and only authoritative current-run outcomes can close a cell.
12. Run scoped tests, migration identity proof, Node/browser tests, full
    precommit, then a new authorized Test1 Enumeration. Inspect
    `run_tree.py --full --db` and DB truth; iterate until worklist, evidence,
    outcomes, denominator, and final gate agree.
